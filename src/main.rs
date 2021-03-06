#[macro_use]
extern crate clap;

#[macro_use]
extern crate failure;

#[macro_use]
extern crate hyper;

#[macro_use]
extern crate serde_derive;

#[macro_use]
extern crate structopt;

extern crate prettytable;
extern crate reqwest;
extern crate rustyline;
extern crate serde_json;

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path;
use std::process::{ChildStdin, Command, ExitStatus, Stdio};

use rustyline::Editor;
use structopt::StructOpt;

mod context;
mod presto;

use context::{Context, OutputFormat};

const VELOCE_BANNER: &'static str = r##"
           _
__   _____| | ___   ___ ___
\ \ / / _ \ |/ _ \ / __/ _ \
 \ V /  __/ | (_) | (_|  __/
  \_/ \___|_|\___/ \___\___|
"##;

static PRESTO_KEYWORDS: [&str; 23] = [
    "ASC", "CASE", "COALESCE", "COUNT", "DESC", "DESCRIBE", "ELSE", "END", "FROM", "GROUP BY",
    "HAVING", "IF", "IN", "IS", "LIMIT", "MAX", "MIN", "NOT", "NULL", "ORDER BY", "SELECT", "THEN",
    "WHEN",
];

static PRESTO_BREAK_CHARS: [char; 11] = [' ', '\t', '\n', '"', '\'', '>', '<', '=', ';', '(', ')'];

pub struct VeloceCompleter(BTreeSet<char>);

impl VeloceCompleter {
    fn new() -> VeloceCompleter {
        VeloceCompleter(PRESTO_BREAK_CHARS.iter().cloned().collect())
    }
}

impl rustyline::completion::Completer for VeloceCompleter {
    fn complete(&self, line: &str, pos: usize) -> rustyline::Result<(usize, Vec<String>)> {
        let (start, word) = rustyline::completion::extract_word(line, pos, &self.0);
        let word = word.to_ascii_uppercase();

        let completions = PRESTO_KEYWORDS
            .iter()
            .filter(|k| k.starts_with(&word))
            .map(|k| k.to_string())
            .collect();

        Ok((start, completions))
    }
}

fn sanitize_query(query: &str) -> &str {
    // remove trailing ; to make the query work
    query.trim_right_matches(';').trim()
}

// the rustyline history file is a line per entry separated by '\n', but we
// support multiline queries which messes things up. Therefore, replace all '\n'
// with '\0' when saving the history and viceversa when reading like postgres or
// git.
fn add_history_entry<C>(editor: &mut Editor<C>, entry: &str)
where
    C: rustyline::completion::Completer,
{
    editor.add_history_entry(entry);
}

fn save_history<C, P>(editor: &mut Editor<C>, path: &P) -> std::io::Result<()>
where
    C: rustyline::completion::Completer,
    P: AsRef<path::Path> + ?Sized,
{
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    let history = editor.get_history();

    for i in 0..history.len() {
        let line = history.get(i).unwrap();
        let line = line.trim().replace('\n', "\0") + "\n";

        writer.write_all(line.as_bytes())?;
    }

    Ok(())
}

fn load_history<C, P>(editor: &mut Editor<C>, filepath: &P) -> std::io::Result<()>
where
    C: rustyline::completion::Completer,
    P: AsRef<path::Path> + ?Sized,
{
    let file = File::open(filepath)?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line?.replace('\0', "\n");
        editor.add_history_entry(&line);
    }

    Ok(())
}

fn main() {
    let ctx = Context::from_args();
    let cli = reqwest::Client::new();

    let base_dir = env::home_dir().unwrap_or_else(path::PathBuf::new);
    let history_file_path = base_dir.join(".veloce_history");

    let mut editor = Editor::<VeloceCompleter>::new();
    editor.set_completer(Some(VeloceCompleter::new()));

    if load_history(&mut editor, &history_file_path).is_err() {
        println!("cannot load history")
    }

    match ctx.query {
        Some(ref query) => {
            run_query(&cli, &ctx, sanitize_query(query).to_string());

            add_history_entry(&mut editor, &query);
        }
        None => {
            println!("{}", VELOCE_BANNER.trim_left_matches('\n'));
            run_interactive(&ctx, &cli, &mut editor);
        }
    };

    save_history(&mut editor, &history_file_path).expect("cannot write history file");
}

fn run_interactive<C>(ctx: &Context, cli: &reqwest::Client, editor: &mut Editor<C>)
where
    C: rustyline::completion::Completer,
{
    let mut query = String::new();

    loop {
        let prompt = if query.is_empty() {
            format!("veloce:{}> ", ctx.schema)
        } else {
            "...> ".to_string()
        };

        match editor.readline(&prompt) {
            Ok(line) => {
                if line.is_empty() {
                    continue;
                }

                query += "\n";
                query += &line;

                if !query.ends_with(';') {
                    continue;
                }

                run_query(&cli, &ctx, sanitize_query(&query).to_string());
                add_history_entry(editor, &query.trim());

                query.clear();
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                query.clear();
            }
            _ => break,
        };
    }
}

fn run_query(cli: &reqwest::Client, ctx: &Context, query: String) {
    let qit = presto::QueryIterator::new(cli, ctx, query);
    let res: presto::Result<Vec<presto::QueryResults>> = qit.collect();

    match res {
        Ok(data) => {
            display_data(ctx, data);
        }
        Err(e) => println!("presto api error: {:?}", e),
    }
}

fn display_data(ctx: &Context, data: Vec<presto::QueryResults>) {
    let mut table = prettytable::Table::new();

    for qres in data {
        if let Some(cols) = qres.columns {
            table.set_titles(cols.iter().map(|c| &c.name).collect());
        }

        for row in qres.data.unwrap_or_else(Vec::new) {
            table.add_row(row.iter().map(|c| c.to_string()).collect());
        }
    }

    if table.is_empty() {
        println!("(0 rows)\n");
        return;
    }

    match ctx.output_format {
        OutputFormat::Csv => {
            let out = std::io::stdout();
            table.to_csv(out).expect("cannot dump csv to stdout");
        }
        OutputFormat::Pretty => {
            let res = with_pager(ctx, |p| {
                let res = table.print(p);
                match res {
                    Err(ref e) if e.kind() != std::io::ErrorKind::BrokenPipe => {
                        println!("pager error: {}", e.description())
                    }
                    _ => (),
                }
            });

            if let Err(e) = res {
                println!("pager error: {}", e.description());
            }
        }
    }
}

fn with_pager<F>(ctx: &Context, f: F) -> Result<ExitStatus, std::io::Error>
where
    F: FnOnce(&mut ChildStdin) -> (),
{
    let (cmd, args) = {
        let mut parts = ctx.pager.split(' ');
        let cmd = parts.next().unwrap_or("less");
        let args: Vec<&str> = parts.collect();

        (cmd, args)
    };

    let mut pager = Command::new(&cmd)
        .args(&args)
        .stdin(Stdio::piped())
        .spawn()?;

    {
        let stdin = pager.stdin.as_mut().expect("cannot happen");
        f(stdin);
    }

    pager.wait()
}
