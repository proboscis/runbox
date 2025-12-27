use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dialoguer::{theme::ColorfulTheme, Input};
use runbox_core::{
    BindingResolver, GitContext, Playlist, PlaylistItem, RunTemplate, Storage, Validator,
};
use std::path::Path;
use std::process::Command;

#[derive(Parser)]
#[command(name = "runbox")]
#[command(about = "Reproducible command execution system")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run from a template
    Run {
        /// Template ID
        #[arg(short, long)]
        template: String,

        /// Variable bindings (key=value)
        #[arg(short, long)]
        binding: Vec<String>,

        /// Skip execution (dry run)
        #[arg(long)]
        dry_run: bool,
    },

    /// Manage templates
    Template {
        #[command(subcommand)]
        command: TemplateCommands,
    },

    /// Manage playlists
    Playlist {
        #[command(subcommand)]
        command: PlaylistCommands,
    },

    /// Show run history
    History {
        /// Limit number of results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },

    /// Show details of a run
    Show {
        /// Run ID
        run_id: String,
    },

    /// Replay a previous run
    Replay {
        /// Run ID
        run_id: String,
    },

    /// Validate a JSON file
    Validate {
        /// Path to JSON file
        path: String,
    },
}

#[derive(Subcommand)]
enum TemplateCommands {
    /// List all templates
    List,
    /// Show template details
    Show { template_id: String },
    /// Create a new template from JSON file
    Create { path: String },
    /// Delete a template
    Delete { template_id: String },
}

#[derive(Subcommand)]
enum PlaylistCommands {
    /// List all playlists
    List,
    /// Show playlist details
    Show { playlist_id: String },
    /// Create a new playlist from JSON file
    Create { path: String },
    /// Add template to playlist
    Add {
        playlist_id: String,
        template_id: String,
        /// Optional label
        #[arg(short, long)]
        label: Option<String>,
    },
    /// Remove template from playlist
    Remove {
        playlist_id: String,
        template_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let storage = Storage::new()?;

    match cli.command {
        Commands::Run {
            template,
            binding,
            dry_run,
        } => cmd_run(&storage, &template, binding, dry_run),
        Commands::Template { command } => match command {
            TemplateCommands::List => cmd_template_list(&storage),
            TemplateCommands::Show { template_id } => cmd_template_show(&storage, &template_id),
            TemplateCommands::Create { path } => cmd_template_create(&storage, &path),
            TemplateCommands::Delete { template_id } => cmd_template_delete(&storage, &template_id),
        },
        Commands::Playlist { command } => match command {
            PlaylistCommands::List => cmd_playlist_list(&storage),
            PlaylistCommands::Show { playlist_id } => cmd_playlist_show(&storage, &playlist_id),
            PlaylistCommands::Create { path } => cmd_playlist_create(&storage, &path),
            PlaylistCommands::Add {
                playlist_id,
                template_id,
                label,
            } => cmd_playlist_add(&storage, &playlist_id, &template_id, label),
            PlaylistCommands::Remove {
                playlist_id,
                template_id,
            } => cmd_playlist_remove(&storage, &playlist_id, &template_id),
        },
        Commands::History { limit } => cmd_history(&storage, limit),
        Commands::Show { run_id } => cmd_show(&storage, &run_id),
        Commands::Replay { run_id } => cmd_replay(&storage, &run_id),
        Commands::Validate { path } => cmd_validate(&path),
    }
}

// === Run Command ===

fn cmd_run(storage: &Storage, template_id: &str, bindings: Vec<String>, dry_run: bool) -> Result<()> {
    let template = storage.load_template(template_id)?;

    // Create interactive callback
    let interactive_callback: Box<dyn Fn(&str, Option<&serde_json::Value>) -> Result<String>> =
        Box::new(|var, default| {
            let prompt = format!("Enter value for '{}'", var);
            let theme = ColorfulTheme::default();
            let mut input = Input::<String>::with_theme(&theme).with_prompt(&prompt);

            if let Some(def) = default {
                let def_str = match def {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => def.to_string(),
                };
                input = input.default(def_str);
            }

            input.interact_text().context("Failed to read input")
        });

    let resolver = BindingResolver::new()
        .with_bindings(bindings)
        .with_interactive(interactive_callback);

    // Get git context
    let git = GitContext::from_current_dir()?;

    // Generate run_id first so we can use it for the patch ref
    let temp_run_id = format!("run_{}", uuid::Uuid::new_v4());
    let code_state = git.build_code_state(&temp_run_id)?;

    // Build run
    let run = resolver.build_run(&template, code_state)?;

    // Validate
    run.validate()?;

    if dry_run {
        println!("Dry run - would execute:");
        println!("{}", serde_json::to_string_pretty(&run)?);
        return Ok(());
    }

    // Save run
    let path = storage.save_run(&run)?;
    println!("Run saved: {}", path.display());

    // Execute
    println!("\nExecuting: {:?}", run.exec.argv);
    let status = Command::new(&run.exec.argv[0])
        .args(&run.exec.argv[1..])
        .current_dir(&run.exec.cwd)
        .envs(&run.exec.env)
        .status()
        .context("Failed to execute command")?;

    if status.success() {
        println!("\nRun completed successfully: {}", run.run_id);
    } else {
        println!("\nRun failed with status: {:?}", status.code());
    }

    Ok(())
}

// === Template Commands ===

fn cmd_template_list(storage: &Storage) -> Result<()> {
    let templates = storage.list_templates()?;

    if templates.is_empty() {
        println!("No templates found.");
        return Ok(());
    }

    println!("{:<30} {:<40}", "ID", "NAME");
    println!("{}", "-".repeat(70));
    for t in templates {
        println!("{:<30} {:<40}", t.template_id, t.name);
    }

    Ok(())
}

fn cmd_template_show(storage: &Storage, template_id: &str) -> Result<()> {
    let template = storage.load_template(template_id)?;
    println!("{}", serde_json::to_string_pretty(&template)?);
    Ok(())
}

fn cmd_template_create(storage: &Storage, path: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path))?;

    // Validate first
    let validator = Validator::new()?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    validator.validate_template(&value)?;

    let template: RunTemplate = serde_json::from_str(&content)?;
    let saved_path = storage.save_template(&template)?;

    println!("Template created: {}", saved_path.display());
    Ok(())
}

fn cmd_template_delete(storage: &Storage, template_id: &str) -> Result<()> {
    storage.delete_template(template_id)?;
    println!("Template deleted: {}", template_id);
    Ok(())
}

// === Playlist Commands ===

fn cmd_playlist_list(storage: &Storage) -> Result<()> {
    let playlists = storage.list_playlists()?;

    if playlists.is_empty() {
        println!("No playlists found.");
        return Ok(());
    }

    println!("{:<30} {:<30} {:<10}", "ID", "NAME", "ITEMS");
    println!("{}", "-".repeat(70));
    for p in playlists {
        println!("{:<30} {:<30} {:<10}", p.playlist_id, p.name, p.items.len());
    }

    Ok(())
}

fn cmd_playlist_show(storage: &Storage, playlist_id: &str) -> Result<()> {
    let playlist = storage.load_playlist(playlist_id)?;
    println!("{}", serde_json::to_string_pretty(&playlist)?);
    Ok(())
}

fn cmd_playlist_create(storage: &Storage, path: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path))?;

    // Validate first
    let validator = Validator::new()?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    validator.validate_playlist(&value)?;

    let playlist: Playlist = serde_json::from_str(&content)?;
    let saved_path = storage.save_playlist(&playlist)?;

    println!("Playlist created: {}", saved_path.display());
    Ok(())
}

fn cmd_playlist_add(
    storage: &Storage,
    playlist_id: &str,
    template_id: &str,
    label: Option<String>,
) -> Result<()> {
    let mut playlist = storage.load_playlist(playlist_id)?;
    playlist.items.push(PlaylistItem {
        template_id: template_id.to_string(),
        label,
    });
    storage.save_playlist(&playlist)?;
    println!("Added {} to {}", template_id, playlist_id);
    Ok(())
}

fn cmd_playlist_remove(storage: &Storage, playlist_id: &str, template_id: &str) -> Result<()> {
    let mut playlist = storage.load_playlist(playlist_id)?;
    let initial_len = playlist.items.len();
    playlist.items.retain(|item| item.template_id != template_id);

    if playlist.items.len() == initial_len {
        anyhow::bail!("Template {} not found in playlist", template_id);
    }

    storage.save_playlist(&playlist)?;
    println!("Removed {} from {}", template_id, playlist_id);
    Ok(())
}

// === History Commands ===

fn cmd_history(storage: &Storage, limit: usize) -> Result<()> {
    let runs = storage.list_runs(limit)?;

    if runs.is_empty() {
        println!("No runs found.");
        return Ok(());
    }

    println!("{:<50} {:<30}", "RUN ID", "COMMAND");
    println!("{}", "-".repeat(80));
    for run in runs {
        let cmd = run.exec.argv.join(" ");
        let cmd_truncated = if cmd.len() > 30 {
            format!("{}...", &cmd[..27])
        } else {
            cmd
        };
        println!("{:<50} {:<30}", run.run_id, cmd_truncated);
    }

    Ok(())
}

fn cmd_show(storage: &Storage, run_id: &str) -> Result<()> {
    let run = storage.load_run(run_id)?;
    println!("{}", serde_json::to_string_pretty(&run)?);
    Ok(())
}

// === Replay Command ===

fn cmd_replay(storage: &Storage, run_id: &str) -> Result<()> {
    let run = storage.load_run(run_id)?;

    println!("Replaying: {}", run_id);
    println!("Command: {:?}", run.exec.argv);
    println!("Commit: {}", run.code_state.base_commit);

    if run.code_state.patch.is_some() {
        println!("Note: This run has a patch - you may need to restore the code state first");
    }

    // Execute
    println!("\nExecuting...");
    let status = Command::new(&run.exec.argv[0])
        .args(&run.exec.argv[1..])
        .current_dir(&run.exec.cwd)
        .envs(&run.exec.env)
        .status()
        .context("Failed to execute command")?;

    if status.success() {
        println!("\nReplay completed successfully");
    } else {
        println!("\nReplay failed with status: {:?}", status.code());
    }

    Ok(())
}

// === Validate Command ===

fn cmd_validate(path: &str) -> Result<()> {
    let validator = Validator::new()?;
    let validation_type = validator.validate_file(Path::new(path))?;
    println!("Valid {} file: {}", validation_type, path);
    Ok(())
}
