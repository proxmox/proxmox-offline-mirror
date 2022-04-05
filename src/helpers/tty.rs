use std::io::Write;

use anyhow::{bail, format_err, Error};
use proxmox_schema::parse_boolean;

pub fn read_string_from_tty(query: &str, default: Option<&str>) -> Result<String, Error> {
    use std::io::{BufRead, BufReader};

    if let Some(default) = default {
        print!("{query} ([{default}]): ");
    } else {
        print!("{query}: ");
    }

    let _ = std::io::stdout().flush();
    let mut line = String::new();

    BufReader::new(std::io::stdin()).read_line(&mut line)?;
    let line = line.trim();
    if line.is_empty() {
        if let Some(default) = default {
            Ok(default.to_string())
        } else {
            // Repeat query
            read_string_from_tty(query, default)
        }
    } else {
        Ok(line.trim().to_string())
    }
}

pub fn read_bool_from_tty(query: &str, default: Option<bool>) -> Result<bool, Error> {
    let default = default.map(|v| if v { "yes" } else { "no" });

    loop {
        match read_string_from_tty(query, default)
            .and_then(|line| parse_boolean(&line.to_lowercase()))
        {
            Ok(val) => {
                return Ok(val);
            }
            Err(err) => {
                eprintln!("Failed to parse response - '{err}'");
            }
        }
    }
}

pub fn read_selection_from_tty<'a, V>(
    query: &str,
    choices: &'a [(V, &str)],
    default: Option<usize>,
) -> Result<&'a V, Error> {
    if choices.is_empty() {
        bail!("Cannot select with empty choices.");
    }

    println!("{query}");
    for (index, (_v, choice)) in choices.iter().enumerate() {
        println!("  {index:2 }) {choice}");
    }
    loop {
        match read_string_from_tty("Choice", default.map(|v| format!("{v}")).as_deref())
            .and_then(|line| line.parse::<usize>().map_err(|err| format_err!("{err}")))
        {
            Ok(choice) => {
                if let Some((v, _choice)) = choices.get(choice) {
                    return Ok(v);
                } else {
                    eprintln!("Not a valid choice.");
                }
            }
            Err(err) => {
                eprintln!("Failed to parse response - {err}");
            }
        };
    }
}
