pub const DEFAULT_PAGER: &'static str = "less --no-init -n --chop-long-lines --quit-if-one-screen";

arg_enum! {
    #[derive(Debug)]
    pub enum OutputFormat {
        Csv,
        Pretty,
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "veloce", about = "simple presto cli")]
pub struct Context {
    /// The Presto server to connect to
    #[structopt(short = "s", long = "server", parse(from_str = "parse_server_url"))]
    pub server: String,

    /// The Presto catalog to use
    #[structopt(short = "c", long = "catalog")]
    pub catalog: String,

    /// The Presto schema to use
    #[structopt(long = "schema")]
    pub schema: String,

    /// The Presto username to use
    #[structopt(short = "u", long = "user", default_value = "veloce")]
    pub user: String,

    /// The Pager to use to show results
    #[structopt(short = "p", long = "pager", env = "VELOCE_PAGER",
                raw(default_value = "DEFAULT_PAGER"))]
    pub pager: String,

    /// The query to run in non iteractive mode. Passing a query string
    /// automatically enables non iteractive mode.
    #[structopt(short = "q", long = "query")]
    pub query: Option<String>,

    /// The output format to use.
    #[structopt(short = "o", long = "output-format", default_value = "pretty",
                raw(possible_values = "&OutputFormat::variants()", case_insensitive = "true"))]
    pub output_format: OutputFormat,
}

pub fn parse_server_url(src: &str) -> String {
    let src = {
        if !src.starts_with("http") {
            "http://".to_string() + src
        } else {
            src.to_string()
        }
    };

    src.trim_right_matches('/').to_string()
}
