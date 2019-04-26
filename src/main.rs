#![allow(dead_code)]

use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use reqwest::{header, Client, IntoUrl, StatusCode, Url};

use indicatif::{HumanDuration, ProgressBar, ProgressStyle};

use rayon::prelude::*;

use structopt::StructOpt;

static DEFAULT_UA: &str =
    "Mozilla/5.0 (X11; Fedora; Linux x86_64; rv:66.0) Gecko/20100101 Firefox/66.0";

static BUFFER_SIZE: usize = 1024 * 1024;

pub struct RemoteFile {
    pub url: Url,
    pub name: PathBuf,
    pub length: usize,
    client: Client,
}

impl RemoteFile {
    fn from(url: &str) -> Option<Self> {
        let url = Url::parse(url).ok()?;
        let client = Client::new();
        let resp = client.head(url)
                         .header(header::USER_AGENT, DEFAULT_UA)
                         .send()
                         .ok()?;
        let url = resp.url();
        let length = resp.content_length()? as usize;
        let mut name = None;
        if resp.status().is_success() {
            if let Some(ctd) = resp.headers().get(header::CONTENT_DISPOSITION) {
                if !ctd.is_empty() {
                    if let Ok(ctd) = ctd.to_str() {
                        let vs: Vec<_> = ctd.split(';').collect();
                        if let Some(fv) = vs.iter().find(|v| v.contains("filename")) {
                            let fvs: Vec<_> = fv.split('=').collect();
                            if fvs.len() == 2 {
                                name = Some(PathBuf::from(fvs[1]));
                            }
                        }
                    }
                }
            } else {
                name = Some(PathBuf::from(url.path()));
            }
        }

        if !name.is_none() {
            Some(RemoteFile { url: url.to_owned(),
                              name: name.unwrap(),
                              length,
                              client })
        } else {
            None
        }
    }

    fn download(&self, w: &mut impl Write) -> Option<&Path> {
        fn get_ranged_data(client: &Client,
                           url: impl IntoUrl,
                           range: (usize, usize))
                           -> Option<Box<[u8]>> {
            let range_content = format!("bytes={}-{}", range.0, range.1 - 1);
            let resp = &mut client.get(url)
                                  .header(header::USER_AGENT, DEFAULT_UA)
                                  .header(header::RANGE, range_content.as_str())
                                  .send()
                                  .ok()?;
            if resp.status() == StatusCode::PARTIAL_CONTENT {
                let mut buffer: Vec<_> = Vec::with_capacity(2 * BUFFER_SIZE);
                resp.copy_to(&mut buffer).ok()?;
                Some(buffer.into_boxed_slice())
            } else {
                None
            }
        }

        let ranges = {
            let mut ranges: Vec<_> =
                (0..(self.length / BUFFER_SIZE)).map(|i| (i * BUFFER_SIZE, (i + 1) * BUFFER_SIZE))
                                                .collect();
            ranges.push((BUFFER_SIZE * (self.length / BUFFER_SIZE), self.length));
            ranges
        };

        let data: Option<Vec<_>> =
            ranges.par_iter()
                  .map(|(from, to)| get_ranged_data(&self.client, self.url.clone(), (*from, *to)))
                  .collect();

        if let Some(buffers) = data {
            let mut saved_length = 0usize;
            for b in buffers {
                w.write_all(&b).ok()?;
                saved_length += b.len();
            }
            assert_eq!(self.length, saved_length);

            Some(self.name.as_path())
        } else {
            None
        }
    }
}

#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(parse(from_os_str))]
    file: PathBuf,

    #[structopt(short = "o",
                long = "output",
                help = "output folder for downloaded files [default: current folder]",
                parse(from_os_str))]
    out: Option<PathBuf>,

    #[structopt(long = "log",
                help = "log file",
                default_value = "downloaded_files.log")]
    log: PathBuf,

    #[structopt(long = "server",
                help = "PDB server url",
                default_value = "https://msdl.microsoft.com/download/symbols")]
    server: String,
}

fn main() -> Result<(), io::Error> {
    let opt = Opt::from_args();

    let started = Instant::now();

    let uris: Vec<_> = BufReader::new(File::open(&opt.file)?).lines()
                                                             .map(|l| l.unwrap())
                                                             .collect();

    let pb = ProgressBar::new(uris.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("[{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}")
        .progress_chars("#>-"));

    let ok_uris: Vec<_> = uris.par_iter()
                              .map(|uri| -> Option<&str> {
                                  let uri = uri.as_str();

                                  let file_path = if let Some(outdir) = &opt.out {
                                      let mut p = outdir.clone();
                                      p.push(uri);
                                      p
                                  } else {
                                      PathBuf::from(uri)
                                  };

                                  fs::create_dir_all(file_path.parent()?).ok()?;

                                  let local_file =
                                      &mut BufWriter::new(File::create(&file_path).ok()?);

                                  let url = format!("{}/{}", opt.server.as_str(), uri);
                                  let remote_file = RemoteFile::from(&url)?;
                                  remote_file.download(local_file)?;

                                  pb.inc(1);

                                  Some(uri)
                              })
                              .collect();
    let ok_uris = ok_uris.iter()
                         .filter(|v| v.is_some())
                         .map(|v| v.unwrap())
                         .collect::<Vec<_>>();

    if !ok_uris.is_empty() {
        let log_file = &mut BufWriter::new(File::create(&opt.log)?);
        writeln!(log_file, "{}", ok_uris.join("\n"))?;

        pb.finish_and_clear();

        println!("Done in {}, {}/{} files successfully downloaded and saved (log: {}).",
                 HumanDuration(started.elapsed()),
                 ok_uris.len(),
                 uris.len(),
                 opt.log.to_string_lossy());
    } else {
        println!("nothing downloaded.");
    }

    Ok(())
}
