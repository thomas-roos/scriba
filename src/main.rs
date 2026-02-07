use anyhow::Result;
use scriba::core::{
    resolve_transcription_mode, AudioFormat, CompressionSettings, LocalModelSize, ScribaConfig,
    TranscriptionMode, WorkflowManager,
};
use scriba::database::Database;
use scriba::enrichment::{EnrichmentService, WorldContext};
use scriba::entities::EntityRegistry;
use scriba::mcp::run_mcp_server;
use scriba::tui::Dashboard;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use structopt::StructOpt;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Print ASCII art with embedded version
fn print_ascii_art() {
    println!(
        r#"
 ███████  ██████ ██████  ██ ██████   █████
██      ██      ██   ██ ██ ██   ██ ██   ██
███████ ██      ██████  ██ ██████  ███████
     ██ ██      ██   ██ ██ ██   ██ ██   ██
███████  ██████ ██   ██ ██ ██████  ██   ██
                                    v{VERSION}
"#
    );
}

#[derive(Debug, StructOpt)]
enum Command {
    Record {
        #[structopt(
            short = "n",
            long = "name",
            help = "Optional name/description for the recording (auto-generated if not provided)"
        )]
        name: Option<String>,
        #[structopt(
            short = "s",
            long = "skip-transcription",
            help = "Skip transcription after recording"
        )]
        skip_transcription: bool,
        #[structopt(
            long = "format",
            help = "Audio format (wav, compressed, mp3)",
            default_value = "wav"
        )]
        format: AudioFormat,
        #[structopt(
            long = "sample-rate",
            help = "Sample rate in Hz",
            default_value = "48000"
        )]
        sample_rate: u32,
        #[structopt(long = "bitrate", help = "Bitrate in kbps for compressed formats")]
        bitrate: Option<u32>,
        #[structopt(
            long = "channels",
            help = "Number of channels (1=mono, 2=stereo)",
            default_value = "1"
        )]
        channels: u16,
        #[structopt(
            long = "speech-optimized",
            help = "Use speech-optimized compression settings"
        )]
        speech_optimized: bool,
        #[structopt(long = "local", help = "Force local transcription (overrides config)")]
        force_local: bool,
        #[structopt(
            long = "model",
            help = "Local Whisper model size (tiny|base|small|medium|large|turbo)"
        )]
        model: Option<LocalModelSize>,
        #[structopt(
            long = "api-key",
            help = "OpenAI API key for API-based transcription (overrides config)"
        )]
        api_key: Option<String>,
    },
    Transcribe {
        #[structopt(
            parse(from_os_str),
            help = "Path to existing recording directory name OR external audio file to import"
        )]
        input: PathBuf,
        #[structopt(
            short = "n",
            long = "name",
            help = "Display name for imported files (auto-generated if not provided)"
        )]
        name: Option<String>,
        #[structopt(long = "local", help = "Force local transcription (overrides config)")]
        force_local: bool,
        #[structopt(
            long = "model",
            help = "Local Whisper model size (tiny|base|small|medium|large|turbo)"
        )]
        model: Option<LocalModelSize>,
        #[structopt(
            long = "api-key",
            help = "OpenAI API key for API-based transcription (overrides config)"
        )]
        api_key: Option<String>,
    },
    Config {
        #[structopt(subcommand)]
        cmd: ConfigCommand,
    },
    Health {
        #[structopt(long = "verbose", help = "Show detailed health information")]
        verbose: bool,
    },
    /// Run the Model Context Protocol (MCP) server over stdio
    Mcp,
    /// Run knowledge extraction on an existing recording
    Enrich {
        #[structopt(help = "Recording directory name to enrich")]
        directory_name: String,
    },
    /// Manage entities (people, organizations)
    Entity {
        #[structopt(subcommand)]
        cmd: EntityCommand,
    },
    /// Manage Scriba's world context (owner profile)
    World {
        #[structopt(subcommand)]
        cmd: WorldCommand,
    },
}

#[derive(Debug, StructOpt)]
enum ConfigCommand {
    Show {
        #[structopt(long = "json", help = "Output in JSON format")]
        json: bool,
    },
    SetLocal {
        #[structopt(help = "Model size (tiny|base|small|medium|large|turbo)")]
        model: LocalModelSize,
    },
    SetApi {
        #[structopt(help = "OpenAI API key")]
        api_key: String,
    },
}

#[derive(Debug, StructOpt)]
enum EntityCommand {
    /// List all entities
    List {
        #[structopt(long = "type", help = "Filter by type (person, organization)")]
        entity_type: Option<String>,
        #[structopt(long = "limit", help = "Limit number of results")]
        limit: Option<i64>,
    },
    /// Show details of a specific entity
    Show {
        #[structopt(help = "Entity ID or name")]
        id_or_name: String,
    },
    /// Rename an entity (old name becomes alias)
    Rename {
        #[structopt(help = "Entity ID")]
        id: i64,
        #[structopt(help = "New name")]
        new_name: String,
    },
    /// Update entity context
    Update {
        #[structopt(help = "Entity ID")]
        id: i64,
        #[structopt(long = "context", help = "New context description")]
        context: String,
    },
    /// Manage entity aliases
    Alias {
        #[structopt(subcommand)]
        cmd: AliasCommand,
    },
    /// Delete an entity
    Delete {
        #[structopt(help = "Entity ID")]
        id: i64,
    },
    /// Merge two entities (source into target)
    Merge {
        #[structopt(help = "Source entity ID (will be deleted)")]
        source_id: i64,
        #[structopt(help = "Target entity ID (will receive merged data)")]
        target_id: i64,
    },
}

#[derive(Debug, StructOpt)]
enum AliasCommand {
    /// Add an alias to an entity
    Add {
        #[structopt(help = "Entity ID")]
        id: i64,
        #[structopt(help = "Alias to add")]
        alias: String,
    },
    /// Remove an alias from an entity
    Remove {
        #[structopt(help = "Entity ID")]
        id: i64,
        #[structopt(help = "Alias to remove")]
        alias: String,
    },
}

#[derive(Debug, StructOpt)]
enum WorldCommand {
    /// Show the current world description
    Show,
    /// Initialize the world with seed content
    Init {
        #[structopt(long = "stdin", help = "Read seed content from stdin")]
        stdin: bool,
    },
    /// Edit the world description (opens in $EDITOR)
    Edit,
    /// Get the path to the world file
    Path,
}

#[derive(Debug, StructOpt)]
#[structopt(name = "scriba", about = "A CLI & TUI for recording and transcribing anything", version = VERSION)]
struct Cli {
    #[structopt(subcommand)]
    command: Option<Command>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::from_args();

    // If no command is provided, launch dashboard directly
    let result = match cli.command {
        None => {
            // Load environment variables from .env file
            dotenv::dotenv().ok();

            // Show ASCII art with version
            print_ascii_art();

            // Launch dashboard directly
            println!("\n╭─ SCRIBA DASHBOARD ─────────────────────────────────────╮");
            println!("│ Launching dashboard interface...                       │");
            println!("╰────────────────────────────────────────────────────────╯\n");

            match Dashboard::new() {
                Ok(mut dashboard) => dashboard.run().await,
                Err(err) => {
                    eprintln!("❌ Failed to open dashboard: {err}");
                    Err(err)
                }
            }
        }
        Some(command) => {
            // Load environment variables for CLI mode
            dotenv::dotenv().ok();

            match command {
                Command::Record {
                    name,
                    skip_transcription,
                    format,
                    sample_rate,
                    bitrate,
                    channels,
                    speech_optimized,
                    force_local,
                    model,
                    api_key,
                } => {
                    // Create compression settings
                    let compression_settings = CompressionSettings {
                        format,
                        sample_rate,
                        bitrate_kbps: bitrate,
                        channels,
                        speech_optimized,
                    };

                    // Load config and resolve transcription mode
                    let config = ScribaConfig::load()?;
                    let transcription_mode = if skip_transcription {
                        None
                    } else {
                        Some(resolve_transcription_mode(
                            force_local,
                            model,
                            api_key,
                            &config,
                        )?)
                    };

                    // Use unified workflow
                    let mut workflow = WorkflowManager::new()?;
                    let _recording = workflow
                        .record_cli(
                            name,
                            Some(compression_settings),
                            !skip_transcription,
                            transcription_mode,
                        )
                        .await?;

                    Ok(())
                }
                Command::Transcribe {
                    input,
                    name,
                    force_local,
                    model,
                    api_key,
                } => {
                    // Load config and resolve transcription mode
                    let config = ScribaConfig::load()?;
                    let transcription_mode =
                        resolve_transcription_mode(force_local, model, api_key, &config)?;

                    let mut workflow = WorkflowManager::new()?;

                    // Detect if input is an external audio file (import + transcribe) or existing recording directory
                    if input.is_file() && input.extension().is_some() {
                        // External audio file - import and transcribe using unified workflow
                        println!("📁 Detected external audio file, importing and transcribing...");
                        let _recording = workflow
                            .complete_import_workflow(&input, name, Some(transcription_mode))
                            .await?;
                        println!("🎉 Import and transcription complete!");
                    } else {
                        // Existing recording directory - re-transcribe using unified workflow
                        println!("📝 Re-transcribing existing recording...");
                        let directory_name = input.to_string_lossy();
                        workflow
                            .retranscribe_recording(&directory_name, transcription_mode)
                            .await?;
                    }

                    Ok(())
                }
                Command::Config { cmd } => match cmd {
                    ConfigCommand::Show { json } => {
                        let config = ScribaConfig::load()?;
                        if json {
                            println!("{}", serde_json::to_string_pretty(&config)?);
                        } else {
                            match &config.transcription {
                                TranscriptionMode::Local { model_size } => {
                                    println!("Transcription Mode: Local");
                                    println!("Model Size: {}", model_size);
                                }
                                TranscriptionMode::Api { api_key: _ } => {
                                    println!("Transcription Mode: OpenAI API");
                                    println!("API Key: ***configured***");
                                }
                            }
                            println!("Audio Settings:");
                            println!("  Sample Rate: {} Hz", config.audio_settings.sample_rate);
                            println!("  Bitrate: {} kbps", config.audio_settings.bitrate);
                            println!("  Channels: {}", config.audio_settings.channels);
                            println!(
                                "  Speech Optimized: {}",
                                config.audio_settings.speech_optimized
                            );
                        }
                        Ok(())
                    }
                    ConfigCommand::SetLocal { model } => {
                        let mut config = ScribaConfig::load()?;
                        config.set_transcription_mode(TranscriptionMode::Local {
                            model_size: model,
                        })?;
                        println!(
                            "✅ Updated transcription mode to local with {} model",
                            model
                        );
                        Ok(())
                    }
                    ConfigCommand::SetApi { api_key } => {
                        let mut config = ScribaConfig::load()?;
                        config.set_transcription_mode(TranscriptionMode::Api { api_key })?;
                        println!("✅ Updated transcription mode to OpenAI API");
                        Ok(())
                    }
                },
                Command::Health { verbose } => {
                    let workflow = WorkflowManager::new()?;
                    let health_status = workflow.health_check()?;

                    if verbose {
                        health_status.print_report();
                    } else {
                        if health_status.is_healthy() {
                            println!("✅ Scriba is healthy");
                        } else {
                            println!("❌ Scriba has issues - run with --verbose for details");
                            std::process::exit(1);
                        }
                    }

                    Ok(())
                }
                Command::Mcp => {
                    // Run MCP server on stdio
                    run_mcp_server().await
                }
                Command::Enrich { directory_name } => {
                    println!("🧠 Running knowledge extraction on: {}", directory_name);
                    let mut workflow = WorkflowManager::new()?;
                    workflow.enrich_existing_recording(&directory_name, true).await?;
                    Ok(())
                }
                Command::Entity { cmd } => {
                    let mut db = Database::new()?;
                    let mut registry = EntityRegistry::new(&mut db);

                    match cmd {
                        EntityCommand::List { entity_type, limit } => {
                            let entities =
                                registry.list_entities(entity_type.as_deref(), limit)?;
                            if entities.is_empty() {
                                println!("No entities found.");
                            } else {
                                println!(
                                    "\n{:<4} {:<12} {:<20} {:<30} {:<8}",
                                    "ID", "Type", "Name", "Aliases", "Mentions"
                                );
                                println!("{}", "-".repeat(80));
                                for entity in entities {
                                    let aliases = entity.aliases_list().join(", ");
                                    let aliases_display = if aliases.len() > 28 {
                                        format!("{}...", &aliases[..25])
                                    } else if aliases.is_empty() {
                                        "-".to_string()
                                    } else {
                                        aliases
                                    };
                                    println!(
                                        "{:<4} {:<12} {:<20} {:<30} {:<8}",
                                        entity.id.unwrap_or(0),
                                        entity.entity_type,
                                        entity.canonical_name,
                                        aliases_display,
                                        entity.mention_count
                                    );
                                }
                            }
                            Ok(())
                        }
                        EntityCommand::Show { id_or_name } => {
                            let entity = if let Ok(id) = id_or_name.parse::<i64>() {
                                registry.get_entity(id)?
                            } else {
                                registry.get_entity_by_name(&id_or_name)?
                            };

                            if let Some(entity) = entity {
                                println!("\n╭─ ENTITY ─────────────────────────────────────────╮");
                                println!("│ ID:       {:<40}│", entity.id.unwrap_or(0));
                                println!("│ Type:     {:<40}│", entity.entity_type);
                                println!("│ Name:     {:<40}│", entity.canonical_name);
                                let aliases = entity.aliases_list().join(", ");
                                let aliases_display =
                                    if aliases.is_empty() { "-".to_string() } else { aliases };
                                println!("│ Aliases:  {:<40}│", aliases_display);
                                println!("│ Mentions: {:<40}│", entity.mention_count);
                                println!("├──────────────────────────────────────────────────┤");
                                if let Some(ctx) = &entity.context {
                                    println!("│ Context:                                         │");
                                    // Word-wrap context to fit
                                    for line in ctx.chars().collect::<Vec<_>>().chunks(48) {
                                        let s: String = line.iter().collect();
                                        println!("│   {:<47}│", s);
                                    }
                                } else {
                                    println!("│ Context:  (none)                                 │");
                                }
                                println!("╰──────────────────────────────────────────────────╯");
                            } else {
                                println!("Entity not found: {}", id_or_name);
                            }
                            Ok(())
                        }
                        EntityCommand::Rename { id, new_name } => {
                            if let Some(entity) = registry.get_entity(id)? {
                                let old_name = entity.canonical_name.clone();
                                registry.rename_entity(id, &new_name)?;
                                println!(
                                    "✅ Renamed entity {} from '{}' to '{}'",
                                    id, old_name, new_name
                                );
                                println!("   (old name '{}' added as alias)", old_name);
                            } else {
                                println!("Entity not found: {}", id);
                            }
                            Ok(())
                        }
                        EntityCommand::Update { id, context } => {
                            if registry.get_entity(id)?.is_some() {
                                registry.update_entity_context(id, &context)?;
                                println!("✅ Updated context for entity {}", id);
                            } else {
                                println!("Entity not found: {}", id);
                            }
                            Ok(())
                        }
                        EntityCommand::Alias { cmd: alias_cmd } => match alias_cmd {
                            AliasCommand::Add { id, alias } => {
                                if registry.get_entity(id)?.is_some() {
                                    registry.add_entity_alias(id, &alias)?;
                                    println!("✅ Added alias '{}' to entity {}", alias, id);
                                } else {
                                    println!("Entity not found: {}", id);
                                }
                                Ok(())
                            }
                            AliasCommand::Remove { id, alias } => {
                                if registry.get_entity(id)?.is_some() {
                                    registry.remove_entity_alias(id, &alias)?;
                                    println!("✅ Removed alias '{}' from entity {}", alias, id);
                                } else {
                                    println!("Entity not found: {}", id);
                                }
                                Ok(())
                            }
                        },
                        EntityCommand::Delete { id } => {
                            if let Some(entity) = registry.get_entity(id)? {
                                registry.delete_entity(id)?;
                                println!(
                                    "✅ Deleted entity {}: {}",
                                    id, entity.canonical_name
                                );
                            } else {
                                println!("Entity not found: {}", id);
                            }
                            Ok(())
                        }
                        EntityCommand::Merge {
                            source_id,
                            target_id,
                        } => {
                            let source = registry.get_entity(source_id)?;
                            let target = registry.get_entity(target_id)?;

                            match (source, target) {
                                (Some(src), Some(tgt)) => {
                                    registry.merge_entities(source_id, target_id)?;
                                    println!(
                                        "✅ Merged '{}' into '{}'",
                                        src.canonical_name, tgt.canonical_name
                                    );
                                    println!(
                                        "   '{}' added as alias, mentions transferred",
                                        src.canonical_name
                                    );
                                }
                                (None, _) => println!("Source entity not found: {}", source_id),
                                (_, None) => println!("Target entity not found: {}", target_id),
                            }
                            Ok(())
                        }
                    }
                }
                Command::World { cmd } => match cmd {
                    WorldCommand::Show => {
                        let world = WorldContext::load()?;
                        if world.has_content() {
                            println!("\n📍 {}\n", world.path.display());
                            if let Some(data) = world.parsed() {
                                // Structured display
                                println!("Owner: {} ({})", data.owner.name, data.owner.role);
                                if !data.owner.organization.is_empty() {
                                    println!("Organization: {}", data.owner.organization);
                                }
                                if !data.owner.location.is_empty() {
                                    println!("Location: {}", data.owner.location);
                                }
                                if !data.people.is_empty() {
                                    println!("\nPeople:");
                                    for p in &data.people {
                                        if p.relationship.is_empty() {
                                            println!("  - {}", p.name);
                                        } else {
                                            println!("  - {} ({})", p.name, p.relationship);
                                        }
                                    }
                                }
                                if !data.organizations.is_empty() {
                                    println!("\nOrganizations:");
                                    for o in &data.organizations {
                                        if o.description.is_empty() {
                                            println!("  - {}", o.name);
                                        } else {
                                            println!("  - {} — {}", o.name, o.description);
                                        }
                                    }
                                }
                                if !data.interests.is_empty() {
                                    println!("\nInterests: {}", data.interests.join(", "));
                                }
                                if !data.projects.is_empty() {
                                    println!("\nProjects:");
                                    for p in &data.projects {
                                        if p.description.is_empty() {
                                            println!("  - {}", p.name);
                                        } else {
                                            println!("  - {} — {}", p.name, p.description);
                                        }
                                    }
                                }
                                if !data.beliefs.is_empty() {
                                    println!("\nBeliefs:");
                                    for b in &data.beliefs {
                                        println!("  - {}", b);
                                    }
                                }
                            } else {
                                // Legacy or raw content
                                println!("{}", world.content);
                            }
                        } else {
                            println!("🌍 No world context configured yet.");
                            println!("\nTo initialize your world, run:");
                            println!("  scriba world init");
                            println!("\nOr create the file directly at:");
                            println!("  {}", WorldContext::file_path().display());
                        }
                        Ok(())
                    }
                    WorldCommand::Init { stdin } => {
                        if WorldContext::exists() {
                            println!("⚠️ World file already exists at:");
                            println!("   {}", WorldContext::file_path().display());
                            println!("\nUse 'scriba world edit' to modify it.");
                            return Ok(());
                        }

                        let seed_content = if stdin {
                            // Read from stdin
                            println!("Reading world seed from stdin...");
                            let mut content = String::new();
                            io::stdin().read_to_string(&mut content)?;
                            content
                        } else {
                            // Interactive prompt
                            println!("🌍 Initialize Scriba's World\n");
                            println!("Tell Scriba about yourself. This context helps with:");
                            println!("  • Better entity recognition (company names, people)");
                            println!("  • More accurate summaries and titles");
                            println!("  • Understanding your conversations\n");
                            println!("Example:");
                            println!("  I'm Giovanni, co-founder of Exein, a cybersecurity startup.");
                            println!("  Variations like 'Exane', 'Xane' in transcripts refer to Exein.");
                            println!("  I work closely with Luca (CTO) and Gianni (co-founder).\n");
                            print!("Enter your world description (press Enter twice to finish):\n> ");
                            io::stdout().flush()?;

                            let mut lines = Vec::new();
                            let stdin = io::stdin();
                            loop {
                                let mut line = String::new();
                                stdin.read_line(&mut line)?;
                                if line.trim().is_empty() {
                                    break;
                                }
                                lines.push(line);
                                print!("> ");
                                io::stdout().flush()?;
                            }
                            lines.join("")
                        };

                        if seed_content.trim().is_empty() {
                            println!("❌ No content provided. World not initialized.");
                            return Ok(());
                        }

                        let config = ScribaConfig::load()?;
                        let service = EnrichmentService::new(
                            &config.enrichment.ollama_endpoint,
                            &config.enrichment.ollama_model,
                        );

                        // Try to convert seed to structured JSON via LLM
                        println!("\n🔍 Building structured world profile...");

                        let world_json = match service.health_check().await {
                            Ok(_) => {
                                match service.extract_world_seed(&seed_content).await {
                                    Ok(world_data) => {
                                        println!("   ✅ Structured profile created");
                                        match world_data.to_json() {
                                            Ok(json) => json,
                                            Err(_) => seed_content.clone(),
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("   ⚠️ Could not structure profile: {}", e);
                                        println!("   Using raw seed text instead.");
                                        seed_content.clone()
                                    }
                                }
                            }
                            Err(_) => {
                                println!("   ⚠️ Ollama not available - saving raw seed text.");
                                println!("   Make sure Ollama is running with model '{}'.", config.enrichment.ollama_model);
                                seed_content.clone()
                            }
                        };

                        WorldContext::initialize(&world_json)?;
                        println!("\n✅ World initialized at:");
                        println!("   {}", WorldContext::file_path().display());

                        // Extract entities from the world description
                        println!("\n🔍 Extracting entities from your world description...");

                        match service.health_check().await {
                            Ok(_) => {
                                match service.extract_world_entities(&seed_content).await {
                                    Ok(extraction) => {
                                        let mut db = Database::new()?;
                                        let mut registry = EntityRegistry::new(&mut db);

                                        let mut created_count = 0;

                                        // Create person entities
                                        for person in &extraction.people {
                                            let context = if person.is_owner {
                                                format!("Owner of this Scriba instance. {}", person.context)
                                            } else {
                                                person.context.clone()
                                            };

                                            match registry.create_entity(
                                                "person",
                                                &person.name,
                                                Some(&context),
                                            ) {
                                                Ok(entity) => {
                                                    // Add aliases
                                                    if let Some(entity_id) = entity.id {
                                                        for alias in &person.aliases {
                                                            let _ = registry.add_entity_alias(entity_id, alias);
                                                        }
                                                    }
                                                    println!("   ✅ Created person: {} {}",
                                                        person.name,
                                                        if person.is_owner { "(owner)" } else { "" }
                                                    );
                                                    created_count += 1;
                                                }
                                                Err(e) => {
                                                    eprintln!("   ⚠️ Failed to create {}: {}", person.name, e);
                                                }
                                            }
                                        }

                                        // Create organization entities
                                        for org in &extraction.organizations {
                                            match registry.create_entity(
                                                "organization",
                                                &org.name,
                                                Some(&org.context),
                                            ) {
                                                Ok(entity) => {
                                                    // Add aliases
                                                    if let Some(entity_id) = entity.id {
                                                        for alias in &org.aliases {
                                                            let _ = registry.add_entity_alias(entity_id, alias);
                                                        }
                                                    }
                                                    println!("   ✅ Created organization: {}", org.name);
                                                    created_count += 1;
                                                }
                                                Err(e) => {
                                                    eprintln!("   ⚠️ Failed to create {}: {}", org.name, e);
                                                }
                                            }
                                        }

                                        println!("\n🎉 Created {} entities from your world description.", created_count);
                                    }
                                    Err(e) => {
                                        eprintln!("\n⚠️ Could not extract entities: {}", e);
                                        println!("   You can manually create them with 'scriba entity' commands.");
                                    }
                                }
                            }
                            Err(_) => {}
                        }

                        println!("\nScriba will now use this context for all extractions.");
                        println!("The world will evolve automatically as you add recordings.");
                        Ok(())
                    }
                    WorldCommand::Edit => {
                        let path = WorldContext::file_path();

                        // Create file with template if it doesn't exist
                        if !path.exists() {
                            let template = "# Scriba's World\n\n\
                                Tell Scriba about yourself here. This helps with better\n\
                                entity recognition and understanding your conversations.\n\n\
                                Example:\n\
                                I'm [Your Name], [your role] at [your company].\n\
                                [Add context about people you work with, projects, etc.]\n";
                            std::fs::write(&path, template)?;
                        }

                        // Open in editor
                        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nano".to_string());
                        let status = std::process::Command::new(&editor)
                            .arg(&path)
                            .status()?;

                        if status.success() {
                            println!("✅ World file saved.");
                        } else {
                            println!("⚠️ Editor exited with non-zero status.");
                        }
                        Ok(())
                    }
                    WorldCommand::Path => {
                        println!("{}", WorldContext::file_path().display());
                        Ok(())
                    }
                },
            }
        }
    };

    if let Err(err) = result {
        eprintln!("An error happened: {err}");
    }

    Ok(())
}
