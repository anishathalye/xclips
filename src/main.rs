use std::fs::File;
use std::io::{self, BufRead};
use std::path::PathBuf;
use std::process::Command;
use std::process;
use std::str::FromStr;

use lazy_static::lazy_static;
use regex::Regex;
use structopt::StructOpt;

// dummy comment

#[derive(StructOpt, Debug)]
#[structopt(name = "xclips")]
struct Opt {
    #[structopt(short = "f", long = "timestamps-file", parse(from_os_str))]
    timestamps_file: Option<PathBuf>,

    #[structopt(short = "c", long = "clip")]
    clip: Vec<String>,

    #[structopt(short = "o", long = "output")]
    output: Option<PathBuf>,

    #[structopt(name = "FILE", parse(from_os_str))]
    file: PathBuf,
}

#[derive(PartialEq, Eq, Clone, Debug)]
struct ParseErr(&'static str);

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
struct Timestamp {
    seconds: u32,
    milliseconds: u32,
}

impl FromStr for Timestamp {
    type Err = ParseErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref RE_S_MS: Regex = Regex::new(r"^(\d+)\.?(\d{1,3})?$").unwrap();
            static ref RE_M_S_MS: Regex = Regex::new(r"^(\d+):(\d{2})\.?(\d{1,3})?$").unwrap();
            static ref RE_H_M_S_MS: Regex = Regex::new(r"^(\d+):(\d{2}):(\d{2})\.?(\d{1,3})?$").unwrap();
        }
        fn parse_ms<'a>(ms: Option<regex::Match<'a>>) -> u32 {
            ms.map_or(0, |v| {
                let n: u32  = v.as_str().parse().unwrap();
                n * (10_u32.pow(3-v.range().len() as u32))
            })
        }
        if let Some(captures) = RE_S_MS.captures(s) {
            Ok(Timestamp {
                seconds: captures[1].parse().unwrap(),
                milliseconds: parse_ms(captures.get(2)),
            })
        } else if let Some(captures) = RE_M_S_MS.captures(s) {
            let m: u32 = captures[1].parse().unwrap();
            let s: u32 = captures[2].parse().unwrap();
            Ok(Timestamp {
                seconds: 60 * m + s,
                milliseconds: parse_ms(captures.get(3)),
            })
        } else if let Some(captures) = RE_H_M_S_MS.captures(s) {
            let h: u32 = captures[1].parse().unwrap();
            let m: u32 = captures[2].parse().unwrap();
            let s: u32 = captures[3].parse().unwrap();
            Ok(Timestamp {
                seconds: 60*60*h + 60*m + s,
                milliseconds: parse_ms(captures.get(4)),
            })
        } else {
            Err(ParseErr("not a valid timestamp"))
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Debug)]
struct Span {
    start: Timestamp,
    end: Timestamp,
}

impl FromStr for Span {
    type Err = ParseErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"^(.*)-(.*)$").unwrap();
        }
        let captures = match RE.captures(s) {
            None => { return Err(ParseErr("doesn't contain a dash")) }
            Some(c) => c
        };
        let start: Timestamp = captures[1].parse()?;
        let end: Timestamp = captures[2].parse()?;
        if start > end {
            return Err(ParseErr("end is before start"))
        }
        Ok(Span{start, end})
    }
}

fn main() {
    let opt = Opt::from_args();

    // get spans
    let mut spans: Vec<Span> = Vec::new();
    // get all clips from the file
    if let Some(ref path) = opt.timestamps_file {
        let file = File::open(path).unwrap_or_else(|_| {
            eprintln!("cannot open file: {}", path.as_os_str().to_str().unwrap());
            process::exit(1);
        });
        for line in io::BufReader::new(file).lines() {
            if let Ok(line) = line {
                let span: Span = line.parse().unwrap_or_else(|_| {
                    eprintln!("cannot parse {} as a time span", line);
                    process::exit(1);
                });
                spans.push(span)
            } else {
                eprintln!("error reading file: {}", path.as_os_str().to_str().unwrap());
                process::exit(1);
            }
        }
    }
    // get all clips from command-line arguments
    for clip in &opt.clip {
        let span: Span = clip.parse().unwrap_or_else(|_| {
            eprintln!("cannot parse {} as a time span", clip);
            process::exit(1);
        });
        spans.push(span)
    }
    spans.sort();

    let input_file = opt.file.clone().into_os_string().into_string().unwrap();

    // get info to prepare output filename
    let input_re = Regex::new(r"^(.*)\.(.*)$").unwrap();
    let output = opt.output.unwrap_or(opt.file);
    let captures = input_re.captures(output.as_os_str().to_str().unwrap()).unwrap_or_else(|| {
        eprintln!("output filename does not have a file extension");
        process::exit(1);
    });
    let base = &captures[1];
    let ext = &captures[2];
    let ndigits = log10_ceil(spans.len());

    for (i, span) in spans.iter().enumerate() {
        let output_filename = if spans.len() == 1 {
            format!("{}_clip.{}", base, ext)
        } else {
            format!("{}_clip{:0width$}.{}", base, i, ext, width = ndigits)
        };

        let seek = format!("{}.{:03}", span.start.seconds, span.start.milliseconds);
        let time_total_ms =
            ((span.end.seconds as u64) * 1000 + (span.end.milliseconds as u64)) -
            (span.start.seconds as u64) * 1000 + (span.start.milliseconds as u64);
        let time_ms = time_total_ms % 1000;
        let time_s = time_total_ms / 1000;
        let time = format!("{}.{:03}", time_s, time_ms);
        
        let status = Command::new("ffmpeg")
            .args(["-ss", &seek, "-i", &input_file, "-t", &time, "-c", "copy", &output_filename])
            .status()
            .unwrap_or_else(|_| {
                eprintln!("failed to spawn ffmpeg");
                process::exit(1);
            });
        if !status.success() {
            eprintln!("ffmpeg command returned non-zero exit status");
            process::exit(1);
        }
    }
}

fn log10_ceil(mut n: usize) -> usize {
    let mut digits = 1;
    while n > 10 {
        n /= 10;
        digits += 1;
    }
    digits
}
