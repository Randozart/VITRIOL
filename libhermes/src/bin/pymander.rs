//! `pymander` CLI — the Rust replacement for `libvitriol/pymander.py`'s
//! command line (`vitriol pymander <cmd>`). Subcommands: list, ingest, nodes,
//! search, select, active, doctrine, promote.

use std::sync::Arc;

use libhermes::pymander;
use libhermes::Hermes;

fn memory_root() -> std::path::PathBuf {
    Hermes::default_root()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let h = Arc::new(Hermes::new(&memory_root()));
    let code = match args.first().map(|s| s.as_str()) {
        Some("list") => cmd_list(h.root()),
        Some("ingest") => cmd_ingest(h.root(), &args[1..]),
        Some("nodes") => cmd_nodes(h.root(), &args[1..]),
        Some("search") => cmd_search(h.root(), &args[1..]),
        Some("select") => cmd_select(h.root(), &args[1..]),
        Some("active") => cmd_active(h.root(), &args[1..]),
        Some("doctrine") => cmd_doctrine(h.root(), &args[1..]),
        Some("promote") => cmd_promote(h.root(), &args[1..]),
        _ => {
            eprintln!("usage: pymander <list|ingest|nodes|search|select|active|doctrine|promote>");
            2
        }
    };
    std::process::exit(code);
}

fn cmd_list(root: &std::path::Path) -> i32 {
    println!(
        "{}",
        serde_json::to_string_pretty(&pymander::list_domains(root)).unwrap()
    );
    0
}

fn cmd_ingest(root: &std::path::Path, args: &[String]) -> i32 {
    let (domain, file) = match (args.first(), args.get(1)) {
        (Some(d), Some(f)) => (d.clone(), f.clone()),
        _ => {
            eprintln!("usage: pymander ingest <domain> <file|-|--rev REV>");
            return 2;
        }
    };
    let mut rev = String::new();
    if args.len() >= 4 && args[2] == "--rev" {
        rev = args[3].clone();
    }
    if rev.is_empty() {
        rev = pymander::repo_rev(&file);
    }
    let md = if file == "-" {
        use std::io::Read;
        let mut buf = String::new();
        let _ = std::io::stdin().read_to_string(&mut buf);
        buf
    } else {
        std::fs::read_to_string(&file).unwrap_or_default()
    };
    let h = Arc::new(Hermes::new(root));
    match pymander::ingest_markdown(&h, &domain, &md, &rev) {
        Ok(res) => {
            println!("{}", serde_json::to_string_pretty(&res).unwrap());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_nodes(root: &std::path::Path, args: &[String]) -> i32 {
    let Some(domain) = args.first() else {
        eprintln!("usage: pymander nodes <domain>");
        return 2;
    };
    let h = Arc::new(Hermes::new(root));
    match pymander::list_nodes(&h, domain) {
        Ok(nodes) => {
            println!("{}", serde_json::to_string_pretty(&nodes).unwrap());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_search(root: &std::path::Path, args: &[String]) -> i32 {
    let (domain, query) = match (args.first(), args.get(1)) {
        (Some(d), Some(q)) => (d.clone(), q.clone()),
        _ => {
            eprintln!("usage: pymander search <domain> <query> [--top-k N]");
            return 2;
        }
    };
    let mut top_k = 5usize;
    if args.len() >= 4 && args[2] == "--top-k" {
        top_k = args[3].parse().unwrap_or(5);
    }
    let h = Arc::new(Hermes::new(root));
    match pymander::search(&h, &domain, &query, top_k) {
        Ok(hits) => {
            println!("{}", serde_json::to_string_pretty(&hits).unwrap());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_select(root: &std::path::Path, args: &[String]) -> i32 {
    let Some((pid, rest)) = args.split_first() else {
        eprintln!("usage: pymander select <project_id> <domains...>");
        return 2;
    };
    let domains: Vec<String> = rest.to_vec();
    match pymander::set_selection(root, pid, &domains) {
        Ok(res) => {
            println!("{}", serde_json::to_string_pretty(&res).unwrap());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cmd_active(root: &std::path::Path, args: &[String]) -> i32 {
    let Some(pid) = args.first() else {
        eprintln!("usage: pymander active <project_id>");
        return 2;
    };
    println!(
        "{}",
        serde_json::to_string(&pymander::get_selection(root, pid)).unwrap()
    );
    0
}

fn cmd_doctrine(root: &std::path::Path, args: &[String]) -> i32 {
    let Some(pid) = args.first() else {
        eprintln!("usage: pymander doctrine <project_id> [--query Q] [--budget N]");
        return 2;
    };
    let mut query = String::new();
    let mut budget = 3000usize;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--query" => {
                if let Some(q) = args.get(i + 1) {
                    query = q.clone();
                    i += 2;
                    continue;
                }
            }
            "--budget" => {
                if let Some(b) = args.get(i + 1) {
                    budget = b.parse().unwrap_or(3000);
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let h = Arc::new(Hermes::new(root));
    let opts = libhermes::pymander::DoctrineOpts {
        query,
        budget_tokens: budget,
        top_k: 3,
    };
    print!("{}", pymander::build_doctrine(&h, root, pid, &opts));
    0
}

fn cmd_promote(root: &std::path::Path, args: &[String]) -> i32 {
    let Some(action) = args.first() else {
        eprintln!("usage: pymander promote <add|list> [domain] [label] [summary] [--source S]");
        return 2;
    };
    match action.as_str() {
        "add" => {
            let (domain, label, summary) = match (args.get(1), args.get(2), args.get(3)) {
                (Some(d), Some(l), Some(s)) => (d.clone(), l.clone(), s.clone()),
                _ => {
                    eprintln!("usage: pymander promote add <domain> <label> <summary>");
                    return 2;
                }
            };
            let mut source = String::new();
            if args.len() >= 6 && args[4] == "--source" {
                source = args[5].clone();
            }
            match pymander::add_candidate(root, &domain, &label, &summary, &source) {
                Ok(res) => {
                    println!("{}", serde_json::to_string_pretty(&res).unwrap());
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        "list" => {
            let domain = args.get(1).cloned().unwrap_or_default();
            println!(
                "{}",
                serde_json::to_string_pretty(&pymander::list_candidates(root, &domain)).unwrap()
            );
            0
        }
        _ => {
            eprintln!("usage: pymander promote add|list ...");
            2
        }
    }
}
