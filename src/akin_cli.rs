//! akin subcommand - code similarity detection

use akin::{
    Database, PairStatus, CodeUnitRecord, Store,
    OllamaEmbedding, embedding_to_bytes, bytes_to_embedding,
    VectorIndex, VectorIndexConfig,
};
use akin::hook::get_db_path;
use clap::Subcommand;
use lsp::{LanguageAdapter, RustAdapter, SwiftAdapter, TypeScriptAdapter, CodeUnit};
use sha2::{Sha256, Digest};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Subcommand)]
pub enum AkinCommands {
    /// Index project to database
    Index {
        /// Project path
        path: String,
        /// Language (rust, swift, typescript/ts)
        #[arg(short, long, default_value = "rust")]
        lang: String,
        /// Embedding model
        #[arg(short, long, default_value = "bge-m3")]
        model: String,
        /// Minimum function lines
        #[arg(long, default_value = "3")]
        min_lines: u32,
    },
    /// Scan for similar code
    Scan {
        /// Project paths (empty = all indexed)
        paths: Vec<String>,
        /// Scan all indexed projects
        #[arg(short, long)]
        all: bool,
        /// Cross-project only
        #[arg(short = 'x', long)]
        cross_only: bool,
        /// Similarity threshold
        #[arg(short, long, default_value = "0.85")]
        threshold: f32,
    },
    /// Cross-project comparison (LSP mode, no database)
    Compare {
        /// Project A path
        path_a: String,
        /// Project A language (rust, swift, typescript/ts)
        #[arg(long, default_value = "typescript")]
        lang_a: String,
        /// Project B path
        path_b: String,
        /// Project B language (rust, swift, typescript/ts)
        #[arg(long, default_value = "typescript")]
        lang_b: String,
        /// Similarity threshold
        #[arg(short, long, default_value = "0.80")]
        threshold: f32,
    },
    /// Show project status
    Status {
        /// Project path
        path: String,
    },
    /// List indexed projects
    Projects,
    /// List similar pairs
    Pairs {
        /// Filter by status (new, ignored, confirmed, redundant)
        #[arg(short, long, default_value = "new")]
        status: String,
        /// Max results
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },
    /// Ignore a pair
    Ignore {
        /// Code unit A
        unit_a: String,
        /// Code unit B
        unit_b: String,
        /// Reason
        #[arg(short, long)]
        reason: Option<String>,
    },
    /// Group management
    #[command(subcommand)]
    Group(GroupCommands),
}

#[derive(Subcommand)]
pub enum GroupCommands {
    /// Create group
    Create {
        /// Group name
        name: String,
        /// Reason
        #[arg(short, long)]
        reason: String,
        /// Pattern
        #[arg(short, long)]
        pattern: Option<String>,
        /// Project path
        #[arg(short = 'P', long)]
        project: Option<String>,
    },
    /// Add to group
    Add {
        /// Group ID
        group_id: i64,
        /// Qualified names
        qualified_names: Vec<String>,
    },
    /// List groups
    List {
        /// Project path
        #[arg(short = 'P', long)]
        project: Option<String>,
    },
    /// List group members
    Members {
        /// Group ID
        group_id: i64,
    },
}

pub async fn run(cmd: AkinCommands) -> anyhow::Result<()> {
    match cmd {
        AkinCommands::Index { path, lang, model, min_lines } => {
            cmd_index(&path, &lang, &model, min_lines).await
        }
        AkinCommands::Scan { paths, all, cross_only, threshold } => {
            cmd_scan(&paths, all, cross_only, threshold).await
        }
        AkinCommands::Compare { path_a, lang_a, path_b, lang_b, threshold } => {
            cmd_compare(&path_a, &lang_a, &path_b, &lang_b, threshold).await
        }
        AkinCommands::Status { path } => cmd_status(&path),
        AkinCommands::Projects => cmd_projects(),
        AkinCommands::Pairs { status, limit } => cmd_pairs(&status, limit),
        AkinCommands::Ignore { unit_a, unit_b, reason } => {
            cmd_ignore(&unit_a, &unit_b, reason.as_deref())
        }
        AkinCommands::Group(sub) => match sub {
            GroupCommands::Create { name, reason, pattern, project } => {
                cmd_group_create(&name, &reason, pattern.as_deref(), project.as_deref())
            }
            GroupCommands::Add { group_id, qualified_names } => {
                cmd_group_add(group_id, &qualified_names)
            }
            GroupCommands::List { project } => cmd_group_list(project.as_deref()),
            GroupCommands::Members { group_id } => cmd_group_members(group_id),
        },
    }
}

fn ensure_db() -> anyhow::Result<Database> {
    let db_path = get_db_path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(Database::open(&db_path)?)
}

fn ensure_store() -> anyhow::Result<Store> {
    let db_path = get_db_path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(Store::open(&db_path)?)
}

async fn cmd_index(path: &str, lang: &str, model: &str, min_lines: u32) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    let project_name = project_path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("Project: {}", project_path.display());
    println!("Language: {}", lang);
    println!("Model: {}", model);
    println!();

    let mut store = ensure_store()?;
    let project_id = store.db_mut().get_or_create_project(&project_name, project_path.to_str().unwrap(), lang)?;

    println!("Extracting code units...");
    let units = extract_functions_lsp(project_path.to_str().unwrap(), lang).await?;
    println!("Found {} functions", units.len());

    let units: Vec<_> = units.into_iter()
        .filter(|u| (u.range_end - u.range_start) >= min_lines)
        .collect();
    println!("After filter: {} functions (>= {} lines)", units.len(), min_lines);

    if units.is_empty() {
        println!("No matching functions found");
        return Ok(());
    }

    println!("\nGenerating embeddings...");
    let mut embedder = OllamaEmbedding::new(model);
    let mut indexed = 0;

    for (i, unit) in units.iter().enumerate() {
        print!("\r  [{}/{}] {}", i + 1, units.len(), short_name(&unit.qualified_name));

        let content_hash = compute_hash(&unit.body);
        let structure_hash = compute_structure_hash(&unit.body);

        let embedding = if let Ok(Some(cached)) = store.db().get_embedding_by_content_hash(&content_hash) {
            cached
        } else {
            match embedder.embed(&unit.body).await {
                Ok(emb) => embedding_to_bytes(&emb),
                Err(e) => {
                    eprintln!("\nWarning: failed to generate embedding: {}", e);
                    continue;
                }
            }
        };

        let record = CodeUnitRecord {
            qualified_name: unit.qualified_name.clone(),
            project_id,
            file_path: unit.file_path.clone(),
            kind: unit.kind.clone(),
            range_start: unit.range_start,
            range_end: unit.range_end,
            content_hash,
            structure_hash,
            embedding: Some(embedding),
            group_id: None,
        };

        store.upsert_code_unit(&record)?;
        indexed += 1;
    }

    store.save_vector_index()?;

    println!("\n\nIndexed: {} code units", indexed);
    if let Some((size, mem)) = store.vector_index_stats() {
        println!("Vector index: {} entries, {} KB", size, mem / 1024);
    }
    store.db_mut().update_project_indexed_time(project_id)?;

    Ok(())
}

async fn cmd_scan(paths: &[String], all: bool, cross_only: bool, threshold: f32) -> anyhow::Result<()> {
    let t0 = Instant::now();
    let store = ensure_store()?;
    let db = store.db();

    let has_vector_index = store.vector_index_stats().is_some();
    if !has_vector_index {
        println!("Warning: vector index not initialized, using brute force (slow)");
    }

    let project_ids: Vec<i64> = if all || paths.is_empty() {
        let projects = db.get_all_projects()?;
        if projects.is_empty() {
            println!("No indexed projects. Run 'iris akin index <path>' first.");
            return Ok(());
        }
        println!("Scanning {} projects: {}", projects.len(),
            projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "));
        projects.iter().map(|p| p.id).collect()
    } else {
        let mut ids = Vec::new();
        for p in paths {
            let resolved = PathBuf::from(p).canonicalize()?;
            match db.get_project_by_path(resolved.to_str().unwrap())? {
                Some(proj) => ids.push(proj.id),
                None => {
                    println!("Project not indexed: {}", resolved.display());
                    return Ok(());
                }
            }
        }
        ids
    };

    let units = db.get_code_units_by_projects(Some(&project_ids))?;
    println!("Loaded {} code units", units.len());

    if units.len() < 2 {
        println!("Not enough code units to compare");
        return Ok(());
    }

    let units_with_emb: Vec<_> = units.iter()
        .filter_map(|u| {
            u.embedding.as_ref()
                .and_then(|e| bytes_to_embedding(e))
                .map(|emb| (u, emb))
        })
        .collect();
    println!("Valid embeddings: {}", units_with_emb.len());

    if units_with_emb.len() < 2 {
        println!("Not enough valid embeddings");
        return Ok(());
    }

    let name_to_project: HashMap<String, i64> = units.iter()
        .map(|u| (u.qualified_name.clone(), u.project_id))
        .collect();

    let queries: Vec<(usize, &[f32])> = units_with_emb.iter()
        .enumerate()
        .map(|(i, (_, emb))| (i, emb.as_slice().unwrap()))
        .collect();

    print!("Searching...");
    let k = 100;
    let search_results = store.search_batch_parallel(&queries, k, threshold)?;

    let mut new_pairs: Vec<(String, String, f32)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for (query_idx, similar_name, similarity) in search_results {
        let query_name = &units_with_emb[query_idx].0.qualified_name;
        let query_project = units_with_emb[query_idx].0.project_id;

        if &similar_name == query_name {
            continue;
        }

        if cross_only {
            if let Some(&similar_project) = name_to_project.get(&similar_name) {
                if similar_project == query_project {
                    continue;
                }
            }
        }

        let pair = if query_name < &similar_name {
            (query_name.clone(), similar_name.clone())
        } else {
            (similar_name.clone(), query_name.clone())
        };

        if seen.insert(pair.clone()) {
            new_pairs.push((pair.0, pair.1, similarity));
        }
    }

    db.batch_upsert_similar_pairs(&new_pairs, Some("scan"))?;

    println!("\rDone: {} pairs ({:.2}s)", new_pairs.len(), t0.elapsed().as_secs_f32());

    let pairs = db.get_similar_pairs(None, None, threshold)?;

    let pairs: Vec<_> = if cross_only && project_ids.len() > 1 {
        pairs.into_iter().filter(|p| {
            let a_proj = units.iter().find(|u| u.qualified_name == p.unit_a).map(|u| u.project_id);
            let b_proj = units.iter().find(|u| u.qualified_name == p.unit_b).map(|u| u.project_id);
            a_proj != b_proj
        }).collect()
    } else {
        pairs
    };

    println!("\nFound {} similar pairs (threshold: {:.0}%)", pairs.len(), threshold * 100.0);
    println!("{}", "=".repeat(60));

    for (i, pair) in pairs.iter().take(20).enumerate() {
        let file_a = pair.file_a.as_ref().map(|f| Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();
        let file_b = pair.file_b.as_ref().map(|f| Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();

        println!("\n[{}] {:.2}%", i + 1, pair.similarity * 100.0);
        println!("  A: {}:{} {}", file_a, pair.start_a.unwrap_or(0), short_name(&pair.unit_a));
        println!("  B: {}:{} {}", file_b, pair.start_b.unwrap_or(0), short_name(&pair.unit_b));
    }

    if pairs.len() > 20 {
        println!("\n... {} more", pairs.len() - 20);
    }

    Ok(())
}

async fn cmd_compare(path_a: &str, lang_a: &str, path_b: &str, lang_b: &str, threshold: f32) -> anyhow::Result<()> {
    let t0 = Instant::now();

    println!("Cross-project comparison (ANN):");
    println!("  A: {} ({})", path_a, lang_a);
    println!("  B: {} ({})", path_b, lang_b);

    let units_a = extract_functions_lsp(path_a, lang_a).await?;
    println!("Project A: {} functions", units_a.len());

    let units_b = extract_functions_lsp(path_b, lang_b).await?;
    println!("Project B: {} functions", units_b.len());

    if units_a.is_empty() || units_b.is_empty() {
        println!("At least one project has no functions");
        return Ok(());
    }

    println!("\nGenerating embeddings...");
    let mut embedder = OllamaEmbedding::new("bge-m3");
    let mut all_embeddings: Vec<(usize, String, Vec<f32>, bool)> = Vec::new();

    for (i, unit) in units_a.iter().enumerate() {
        print!("\r  A: [{}/{}]", i + 1, units_a.len());
        if let Ok(emb) = embedder.embed(&unit.body).await {
            let vec: Vec<f32> = emb.as_slice().unwrap().to_vec();
            all_embeddings.push((all_embeddings.len(), unit.qualified_name.clone(), vec, true));
        }
    }
    println!();

    for (i, unit) in units_b.iter().enumerate() {
        print!("\r  B: [{}/{}]", i + 1, units_b.len());
        if let Ok(emb) = embedder.embed(&unit.body).await {
            let vec: Vec<f32> = emb.as_slice().unwrap().to_vec();
            all_embeddings.push((all_embeddings.len(), unit.qualified_name.clone(), vec, false));
        }
    }
    println!();

    if all_embeddings.len() < 2 {
        println!("Not enough valid embeddings");
        return Ok(());
    }

    println!("Building ANN index...");
    let dimensions = all_embeddings[0].2.len();
    let config = VectorIndexConfig {
        dimensions,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 64,
    };
    let index = VectorIndex::new(config)?;
    index.reserve(all_embeddings.len())?;

    for (idx, _, emb, _) in &all_embeddings {
        index.add(*idx as u64, emb)?;
    }

    println!("Searching...");
    let k = 50;
    let mut cross_pairs: Vec<(String, String, f32)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    let project_a_indices: HashSet<u64> = all_embeddings.iter()
        .filter(|(_, _, _, is_a)| *is_a)
        .map(|(idx, _, _, _)| *idx as u64)
        .collect();

    for (_idx, name_a, emb, is_a) in &all_embeddings {
        if !*is_a { continue; }

        let results = index.search_filtered(emb, k, |id| !project_a_indices.contains(&id))?;

        for result in results {
            let similarity = result.similarity();
            if similarity < threshold { continue; }

            let name_b = &all_embeddings[result.id as usize].1;
            let pair = if name_a < name_b {
                (name_a.clone(), name_b.clone())
            } else {
                (name_b.clone(), name_a.clone())
            };

            if seen.insert(pair.clone()) {
                cross_pairs.push((pair.0, pair.1, similarity));
            }
        }
    }

    cross_pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    println!("\nFound {} cross-project pairs (threshold: {:.0}%, {:.2}s)",
        cross_pairs.len(), threshold * 100.0, t0.elapsed().as_secs_f32());

    for (i, (unit_a, unit_b, similarity)) in cross_pairs.iter().take(30).enumerate() {
        println!("\n[{}] {:.2}%", i + 1, similarity * 100.0);
        println!("  A: {}", format_name(unit_a));
        println!("  B: {}", format_name(unit_b));
    }

    if cross_pairs.len() > 30 {
        println!("\n... {} more", cross_pairs.len() - 30);
    }

    Ok(())
}

fn cmd_status(path: &str) -> anyhow::Result<()> {
    let project_path = PathBuf::from(path).canonicalize()?;
    let db = ensure_db()?;

    match db.get_project_by_path(project_path.to_str().unwrap())? {
        Some(project) => {
            let stats = db.get_stats(project.id)?;
            println!("Project: {}", project.name);
            println!("Path: {}", project.root_path);
            println!("Language: {}", project.language);
            println!("Last indexed: {}", project.last_indexed_at.unwrap_or_else(|| "never".to_string()));
            println!();
            println!("Code units: {}", stats.total_units);
            println!("Groups: {}", stats.total_groups);
            println!();
            println!("Pairs by status:");
            for (status, count) in &stats.pairs_by_status {
                println!("  {}: {}", status, count);
            }
        }
        None => println!("Project not indexed: {}", project_path.display()),
    }
    Ok(())
}

fn cmd_projects() -> anyhow::Result<()> {
    let db = ensure_db()?;
    let projects = db.get_all_projects()?;

    if projects.is_empty() {
        println!("No indexed projects.");
        return Ok(());
    }

    for project in projects {
        println!("[{}] {}", project.id, project.name);
        println!("    Path: {}", project.root_path);
        println!("    Language: {}", project.language);
        println!("    Last indexed: {}", project.last_indexed_at.unwrap_or_else(|| "never".to_string()));
        println!();
    }
    Ok(())
}

fn cmd_pairs(status: &str, limit: usize) -> anyhow::Result<()> {
    let db = ensure_db()?;
    let pair_status = PairStatus::from_str(status)
        .ok_or_else(|| anyhow::anyhow!("Invalid status: {}", status))?;

    let pairs = db.get_similar_pairs(None, Some(pair_status), 0.0)?;

    println!("Similar pairs (status: {}):\n", status);

    for pair in pairs.iter().take(limit) {
        let file_a = pair.file_a.as_ref().map(|f| Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();
        let file_b = pair.file_b.as_ref().map(|f| Path::new(f).file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_default();

        println!("[{}] {:.2}%", pair.id, pair.similarity * 100.0);
        println!("  A: {}:{} {}", file_a, pair.start_a.unwrap_or(0), short_name(&pair.unit_a));
        println!("  B: {}:{} {}", file_b, pair.start_b.unwrap_or(0), short_name(&pair.unit_b));
        println!();
    }

    if pairs.len() > limit {
        println!("... {} more", pairs.len() - limit);
    }
    Ok(())
}

fn cmd_ignore(unit_a: &str, unit_b: &str, _reason: Option<&str>) -> anyhow::Result<()> {
    let db = ensure_db()?;
    let (a, b) = if unit_a < unit_b { (unit_a, unit_b) } else { (unit_b, unit_a) };

    let pairs = db.get_similar_pairs(None, None, 0.0)?;
    let pair = pairs.iter().find(|p| p.unit_a == a && p.unit_b == b);

    match pair {
        Some(p) => {
            db.update_pair_status(p.id, PairStatus::Ignored)?;
            println!("Ignored pair ({:.2}%):", p.similarity * 100.0);
            println!("  A: {}", a);
            println!("  B: {}", b);
        }
        None => println!("Pair not found."),
    }
    Ok(())
}

fn cmd_group_create(name: &str, reason: &str, pattern: Option<&str>, project: Option<&str>) -> anyhow::Result<()> {
    let db = ensure_db()?;
    let project_path = match project {
        Some(p) => PathBuf::from(p).canonicalize()?,
        None => std::env::current_dir()?,
    };

    let proj = db.get_project_by_path(project_path.to_str().unwrap())?
        .ok_or_else(|| anyhow::anyhow!("Project not indexed: {}", project_path.display()))?;

    let group_id = db.create_group(proj.id, name, Some(reason), pattern)?;

    println!("Created group [{}] '{}'", group_id, name);
    println!("  Reason: {}", reason);
    if let Some(p) = pattern {
        println!("  Pattern: {}", p);
    }
    println!("\nAdd members: iris akin group add {} <qualified_name>", group_id);
    Ok(())
}

fn cmd_group_add(group_id: i64, qualified_names: &[String]) -> anyhow::Result<()> {
    let db = ensure_db()?;
    for qn in qualified_names {
        match db.get_code_unit(qn)? {
            Some(_) => {
                db.add_to_group(qn, group_id)?;
                println!("Added to group {}: {}", group_id, qn);
            }
            None => println!("Warning: code unit not found: {}", qn),
        }
    }
    Ok(())
}

fn cmd_group_list(project: Option<&str>) -> anyhow::Result<()> {
    let db = ensure_db()?;

    let groups = if let Some(p) = project {
        let project_path = PathBuf::from(p).canonicalize()?;
        let proj = db.get_project_by_path(project_path.to_str().unwrap())?
            .ok_or_else(|| anyhow::anyhow!("Project not indexed: {}", project_path.display()))?;
        println!("Groups for {}:", proj.name);
        db.get_groups(proj.id)?
    } else {
        println!("All groups:");
        let mut all_groups = Vec::new();
        for proj in db.get_all_projects()? {
            all_groups.extend(db.get_groups(proj.id)?);
        }
        all_groups
    };

    if groups.is_empty() {
        println!("  (none)");
        return Ok(());
    }

    for g in groups {
        println!("\n[{}] {}", g.id, g.name);
        if let Some(reason) = &g.reason {
            println!("    Reason: {}", reason);
        }
        if let Some(pattern) = &g.pattern {
            println!("    Pattern: {}", pattern);
        }
    }
    Ok(())
}

fn cmd_group_members(group_id: i64) -> anyhow::Result<()> {
    let db = ensure_db()?;
    let all_units = db.get_code_units_by_projects(None)?;
    let members: Vec<_> = all_units.iter()
        .filter(|u| u.group_id == Some(group_id))
        .collect();

    if members.is_empty() {
        println!("Group {} has no members", group_id);
        return Ok(());
    }

    println!("Group {} members:", group_id);
    for unit in members {
        let file_name = Path::new(&unit.file_path).file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        println!("  {}:{} {}", file_name, unit.range_start, short_name(&unit.qualified_name));
    }
    Ok(())
}

async fn extract_functions_lsp(path: &str, lang: &str) -> anyhow::Result<Vec<CodeUnit>> {
    match lang {
        "rust" => {
            let mut adapter = RustAdapter::new(path);
            adapter.start().await?;
            let units = adapter.get_functions().await?;
            adapter.stop()?;
            Ok(units)
        }
        "swift" => {
            let mut adapter = SwiftAdapter::new(path);
            adapter.start().await?;
            let units = adapter.get_functions().await?;
            adapter.stop()?;
            Ok(units)
        }
        "typescript" | "ts" => {
            let mut adapter = TypeScriptAdapter::new(path);
            adapter.start().await?;
            let units = adapter.get_functions().await?;
            adapter.stop()?;
            Ok(units)
        }
        _ => anyhow::bail!("Unsupported language: {}", lang),
    }
}

fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn compute_structure_hash(content: &str) -> String {
    let normalized = content.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    compute_hash(&normalized)
}

fn short_name(name: &str) -> String {
    name.split("::").last().unwrap_or(name).to_string()
}

fn format_name(name: &str) -> String {
    let parts: Vec<&str> = name.splitn(2, "::").collect();
    if parts.len() == 2 {
        let file_part = parts[0];
        let func_part = parts[1];
        let file_name = file_part.rsplit('/').next().unwrap_or(file_part)
            .trim_start_matches("swift:")
            .trim_start_matches("rust:");
        format!("{} ({})", func_part, file_name)
    } else {
        name.to_string()
    }
}
