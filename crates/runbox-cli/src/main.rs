use clap::{Parser, Subcommand};
use anyhow::Result;

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
    /// Create a new template
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
    /// Create a new playlist
    Create { path: String },
    /// Add template to playlist
    Add {
        playlist_id: String,
        template_id: String,
    },
    /// Remove template from playlist
    Remove {
        playlist_id: String,
        template_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { template, binding } => {
            println!("Running template: {} with bindings: {:?}", template, binding);
            // TODO: Implement
        }
        Commands::Template { command } => match command {
            TemplateCommands::List => {
                println!("Listing templates...");
                // TODO: Implement
            }
            TemplateCommands::Show { template_id } => {
                println!("Showing template: {}", template_id);
                // TODO: Implement
            }
            TemplateCommands::Create { path } => {
                println!("Creating template from: {}", path);
                // TODO: Implement
            }
            TemplateCommands::Delete { template_id } => {
                println!("Deleting template: {}", template_id);
                // TODO: Implement
            }
        },
        Commands::Playlist { command } => match command {
            PlaylistCommands::List => {
                println!("Listing playlists...");
                // TODO: Implement
            }
            PlaylistCommands::Show { playlist_id } => {
                println!("Showing playlist: {}", playlist_id);
                // TODO: Implement
            }
            PlaylistCommands::Create { path } => {
                println!("Creating playlist from: {}", path);
                // TODO: Implement
            }
            PlaylistCommands::Add {
                playlist_id,
                template_id,
            } => {
                println!("Adding {} to {}", template_id, playlist_id);
                // TODO: Implement
            }
            PlaylistCommands::Remove {
                playlist_id,
                template_id,
            } => {
                println!("Removing {} from {}", template_id, playlist_id);
                // TODO: Implement
            }
        },
        Commands::History { limit } => {
            println!("Showing last {} runs...", limit);
            // TODO: Implement
        }
        Commands::Show { run_id } => {
            println!("Showing run: {}", run_id);
            // TODO: Implement
        }
        Commands::Replay { run_id } => {
            println!("Replaying run: {}", run_id);
            // TODO: Implement
        }
        Commands::Validate { path } => {
            println!("Validating: {}", path);
            // TODO: Implement
        }
    }

    Ok(())
}
