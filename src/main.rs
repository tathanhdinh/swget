#![allow(dead_code)]

use std::{
    env,
    fs::File,
    io::{BufWriter, Read, Write},
};

use reqwest::{header, Client, StatusCode, Url};

use indicatif::{ProgressBar, ProgressStyle};

use rayon::prelude::*;

static DEFAULT_UA: &str =
    "Mozilla/5.0 (X11; Fedora; Linux x86_64; rv:66.0) Gecko/20100101 Firefox/66.0";

static BUFFER_SIZE: usize = 512 * 1024;

fn get_name(client: &Client, url: Url) -> Option<String> {
    let resp = client.head(url)
                     .header(header::USER_AGENT, DEFAULT_UA)
                     .send()
                     .ok()?;
    if resp.status().is_success() {
        if let Some(ct_disp) = resp.headers().get(header::CONTENT_DISPOSITION) {
            if !ct_disp.is_empty() {
                let name = ct_disp.to_str().ok()?;
                return Some(name.to_owned())
            }
        } else {
            let url = resp.url().as_str();
            let name_pos = url.rfind("/")? + 1;
            return Some(url[name_pos..].to_owned())
        }
    }

    None
}

fn get_length(client: &Client, url: Url) -> Option<usize> {
    let resp = &mut client.get(url)
                          .header(header::USER_AGENT, DEFAULT_UA)
                          .send()
                          .ok()?;
    if resp.status().is_success() {
        Some(resp.content_length()? as usize)
    } else {
        None
    }
}

fn get_ranged_data(client: &Client, url: Url, range: (usize, usize)) -> Option<Box<[u8]>> {
    let range_content = format!("bytes={}-{}", range.0, range.1 - 1);
    let resp = &mut client.get(url)
                          .header(header::USER_AGENT, DEFAULT_UA)
                          .header(header::RANGE, range_content.as_str())
                          .send()
                          .ok()?;
    if resp.status() == StatusCode::PARTIAL_CONTENT {
        let mut buffer = vec![0u8; 2 * (range.1 - range.0)];
        let copied = resp.copy_to(&mut buffer).ok()?;
        buffer.resize(copied as usize, 0u8);
        Some(buffer.into_boxed_slice())
    } else {
        None
    }
}

fn concurrent_download(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let client = &Client::new();
    let name = get_name(client, url.clone())?;
    let length = get_length(client, url.clone())?;

    let ranges = {
        let mut ranges: Vec<_> =
            (0..(length / BUFFER_SIZE)).map(|i| (i * BUFFER_SIZE, (i + 1) * BUFFER_SIZE))
                                       .collect();
        ranges.push((BUFFER_SIZE * (length / BUFFER_SIZE), length));
        ranges
    };

    let pb = ProgressBar::new(length as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .progress_chars("#>-"));

    let data: Option<Vec<_>> = ranges.par_iter()
                                     .map(|(from, to)| {
                                         pb.inc((*to - *from) as u64);
                                         get_ranged_data(client, url.clone(), (*from, *to))
                                     })
                                     .collect();

    if let Some(buffers) = data {
        let file = &mut BufWriter::new(File::create(name.as_str()).ok()?);

        let mut saved_length = 0usize;
        for b in buffers {
            file.write_all(&b).ok()?;
            saved_length += b.len();
        }
        assert_eq!(length, saved_length);

        Some(name)
    } else {
        None
    }
}

fn download(url: &str) -> Option<String> {
    let url = Url::parse(url).ok()?;
    let client = &Client::new();
    let name = get_name(client, url.clone())?;
    let length = get_length(client, url.clone())?;

    let pb = ProgressBar::new(length as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .progress_chars("#>-"));

    let resp = &mut client.get(url)
                          .header(header::USER_AGENT, DEFAULT_UA)
                          .send()
                          .ok()?;

    let buffer = &mut vec![0u8; BUFFER_SIZE];
    let file = &mut BufWriter::new(File::create(name.as_str()).ok()?);

    let mut saved_length = 0usize;
    loop {
        let count = resp.read(buffer).ok()?;
        if count == 0 {
            break
        }
        file.write_all(&buffer[0..count]).ok()?;
        saved_length += count;
        pb.inc(count as u64);
    }
    assert_eq!(length, saved_length);

    Some(name)
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() > 1 {
        let url = args[1].as_str();
        if let Some(file) = concurrent_download(url) {
            println!("file saved to: {}", file.as_str());
        }
    // if let Some(name) = get_name(url) {
    //     println!("remote name: {} (original url: {})", name, url);
    // } else {
    //     println!("remote name not found");
    // }
    } else {
        println!("use: swget url");
    }
}
