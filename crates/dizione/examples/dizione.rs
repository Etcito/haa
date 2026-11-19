//! The parity tool: one text per line in, one answer per line out, in the format of tiny-voca's
//! `js/test/dizione.mjs`, so the output can be compared with dizione's Python or the JS port line by line.
//!
//! `cargo run --release -p dizione --example dizione -- ids|units|words <phonemizer dir or .gguf> texts.txt out.txt`
//!
//! * `ids`: the token ids, a tab, the stress values;
//! * `units`: the clauses as `U:word|kind|attach|source|phones` and `P:mark`, separated by \x1f;
//! * `words`: one word per line: its source, a tab, its phones.

use std::collections::HashMap;
use std::time::Instant;

use dizione::{Part, Phonemizer};

fn show(phones: &[(&str, u8)]) -> String {
    if phones.is_empty() {
        return "-".into();
    }
    phones.iter().map(|(p, s)| format!("{p}/{s}")).collect::<Vec<_>>().join(" ")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: dizione ids|units|words <phonemizer dir or .gguf> texts.txt out.txt");
        std::process::exit(1);
    }
    let started = Instant::now();
    let source = std::path::Path::new(&args[1]);
    let phonemizer = if source.is_dir() {
        let mut files = HashMap::new();
        for entry in std::fs::read_dir(source).expect("data directory") {
            let path = entry.unwrap().path();
            files.insert(path.file_name().unwrap().to_string_lossy().to_string(), std::fs::read(&path).unwrap());
        }
        Phonemizer::from_files(&files)
    } else {
        Phonemizer::from_gguf(&std::fs::read(source).expect("gguf file"))
    }
    .expect("phonemizer data");
    let loaded = Instant::now();
    let text = std::fs::read_to_string(&args[2]).expect("texts");
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let mut out = Vec::with_capacity(lines.len());
    for line in &lines {
        let answer = match args[0].as_str() {
            "ids" => match phonemizer.phonemize(line) {
                Ok(r) => format!(
                    "{}\t{}",
                    r.ids.iter().map(u32::to_string).collect::<Vec<_>>().join(" "),
                    r.stress.iter().map(u8::to_string).collect::<Vec<_>>().join(" ")
                ),
                Err(_) => "ERROR ValueError".into(),
            },
            "units" => phonemizer
                .analyse(line)
                .iter()
                .flat_map(|part| match part {
                    Part::Punct(m) => vec![format!("P:{m}")],
                    Part::Clause(units) => units
                        .iter()
                        .map(|u| format!("U:{}|{}|{}|{}|{}", u.word, u.kind, u.attach as u8, u.source, show(&u.phones)))
                        .collect(),
                })
                .collect::<Vec<_>>()
                .join("\x1f"),
            _ => {
                let (phones, source) = phonemizer.word(line);
                format!("{source}\t{}", show(&phones))
            }
        };
        out.push(answer);
    }
    std::fs::write(&args[3], out.join("\n") + "\n").expect("output");
    let rate = lines.len() as f64 / loaded.elapsed().as_secs_f64();
    eprintln!("{}: {} lines, load {} ms, {rate:.0} lines/s", args[0], lines.len(), (loaded - started).as_millis());
}
