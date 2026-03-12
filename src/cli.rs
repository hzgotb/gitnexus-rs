use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::commands::{
    analyze::{self, AnalyzeOptions},
    augment,
    clean::{self, CleanOptions},
    context::{self, ContextOptions},
    cypher::{self, CypherOptions},
    detect_changes::{self, DetectChangesOptions},
    eval_server::{self, EvalServerOptions},
    impact::{self, ImpactOptions},
    list, mcp,
    query::{self, QueryOptions},
    rename::{self, RenameOptions},
    serve::{self, ServeOptions},
    setup, status,
    wiki::{self, WikiOptions},
};

#[derive(Debug, Parser)]
#[command(name = "gitnexus")]
#[command(about = "GitNexus Rust CLI (migration preview)", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Setup,
    Analyze {
        path: Option<PathBuf>,
        #[arg(short, long)]
        force: bool,
        #[arg(long)]
        embeddings: bool,
    },
    Serve {
        #[arg(short, long)]
        port: Option<u16>,
        #[arg(long)]
        host: Option<String>,
    },
    Mcp,
    List,
    Status,
    Clean {
        #[arg(short, long)]
        force: bool,
        #[arg(long)]
        all: bool,
    },
    Wiki {
        path: Option<PathBuf>,
        #[arg(short, long)]
        force: bool,
        #[arg(long)]
        model: Option<String>,
        #[arg(long = "base-url")]
        base_url: Option<String>,
        #[arg(long = "api-key")]
        api_key: Option<String>,
        #[arg(long)]
        concurrency: Option<u16>,
        #[arg(long)]
        gist: bool,
    },
    Augment {
        pattern: String,
    },
    Query {
        search_query: String,
        #[arg(short, long)]
        repo: Option<String>,
        #[arg(short = 'c', long)]
        context: Option<String>,
        #[arg(short, long)]
        goal: Option<String>,
        #[arg(short, long)]
        limit: Option<u32>,
        #[arg(long)]
        content: bool,
    },
    Context {
        name: Option<String>,
        #[arg(short, long)]
        repo: Option<String>,
        #[arg(short, long)]
        uid: Option<String>,
        #[arg(short, long)]
        file: Option<String>,
        #[arg(long)]
        content: bool,
    },
    Impact {
        target: String,
        #[arg(short, long, default_value = "upstream")]
        direction: String,
        #[arg(short, long)]
        repo: Option<String>,
        #[arg(long)]
        depth: Option<u32>,
        #[arg(long = "include-tests")]
        include_tests: bool,
        #[arg(long = "relation-types", value_delimiter = ',')]
        relation_types: Option<Vec<String>>,
        #[arg(long = "min-confidence")]
        min_confidence: Option<f32>,
    },
    Cypher {
        query: String,
        #[arg(short, long)]
        repo: Option<String>,
    },
    DetectChanges {
        #[arg(long)]
        scope: Option<String>,
        #[arg(long = "base-ref")]
        base_ref: Option<String>,
        #[arg(short, long)]
        repo: Option<String>,
    },
    Rename {
        #[arg(long = "symbol-name")]
        symbol_name: Option<String>,
        #[arg(long = "symbol-uid")]
        symbol_uid: Option<String>,
        #[arg(long = "new-name")]
        new_name: String,
        #[arg(long = "file-path")]
        file_path: Option<String>,
        #[arg(short, long)]
        repo: Option<String>,
        #[arg(long = "apply")]
        apply: bool,
    },
    EvalServer {
        #[arg(short, long)]
        port: Option<u16>,
        #[arg(long = "idle-timeout")]
        idle_timeout: Option<u32>,
    },
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Setup => setup::run(),
        Commands::Analyze {
            path,
            force,
            embeddings,
        } => analyze::run(path.as_deref(), AnalyzeOptions { force, embeddings }),
        Commands::Serve { port, host } => serve::run(ServeOptions { port, host }),
        Commands::Mcp => mcp::run(),
        Commands::List => list::run(),
        Commands::Status => status::run(),
        Commands::Clean { force, all } => clean::run(CleanOptions { force, all }),
        Commands::Wiki {
            path,
            force,
            model,
            base_url,
            api_key,
            concurrency,
            gist,
        } => wiki::run(
            path.as_deref(),
            WikiOptions {
                force,
                model,
                base_url,
                api_key,
                concurrency,
                gist,
            },
        ),
        Commands::Augment { pattern } => augment::run(&pattern),
        Commands::Query {
            search_query,
            repo,
            context,
            goal,
            limit,
            content,
        } => query::run(
            &search_query,
            QueryOptions {
                repo,
                context,
                goal,
                limit,
                content,
            },
        ),
        Commands::Context {
            name,
            repo,
            uid,
            file,
            content,
        } => context::run(
            name.as_deref(),
            ContextOptions {
                repo,
                uid,
                file,
                content,
            },
        ),
        Commands::Impact {
            target,
            direction,
            repo,
            depth,
            include_tests,
            relation_types,
            min_confidence,
        } => impact::run(
            &target,
            ImpactOptions {
                direction,
                repo,
                depth,
                include_tests,
                relation_types,
                min_confidence,
            },
        ),
        Commands::Cypher { query, repo } => cypher::run(&query, CypherOptions { repo }),
        Commands::DetectChanges {
            scope,
            base_ref,
            repo,
        } => detect_changes::run(DetectChangesOptions {
            repo,
            scope,
            base_ref,
        }),
        Commands::Rename {
            symbol_name,
            symbol_uid,
            new_name,
            file_path,
            repo,
            apply,
        } => rename::run(RenameOptions {
            symbol_name,
            symbol_uid,
            new_name,
            file_path,
            repo,
            dry_run: !apply,
        }),
        Commands::EvalServer { port, idle_timeout } => {
            eval_server::run(EvalServerOptions { port, idle_timeout })
        }
    }
}
