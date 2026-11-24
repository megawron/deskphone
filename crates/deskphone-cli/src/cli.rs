use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use deskphone_core::provider::network::get_local_ip_for;
use deskphone_core::{AsteriskProvider, GenericSipProvider, PbxProvider, SwyxProvider};

#[derive(Parser)]
#[command(name = "swyx-client")]
#[command(
    about = "Modular Softphone & PBX Backend Client (Swyx, Generic SIP, Asterisk, FRITZ!Box)",
    long_about = None
)]
struct Cli {
    /// PBX provider: 'swyx' (default), 'sip' (Generic SIP, FRITZ!Box), or 'asterisk' (PJSIP + AMI)
    #[arg(short = 'P', long, default_value = "swyx", global = true, env = "PBX_PROVIDER")]
    provider: String,

    /// PBX server hostname or IP address
    #[arg(short, long, default_value = "127.0.0.1", global = true, env = "PBX_SERVER")]
    server: String,

    /// Swyx CDS SOAP / HTTP port (Swyx only)
    #[arg(short, long, default_value_t = 8094, global = true, env = "SWYX_CDS_PORT")]
    cds_port: u16,

    /// Swyx CDS REST API HTTPS port (Swyx only)
    #[arg(long, default_value_t = 9100, global = true, env = "SWYX_REST_PORT")]
    rest_port: u16,

    /// SIP Signaling Port (UDP/TCP)
    #[arg(long, default_value_t = 5060, global = true, env = "SIP_PORT")]
    sip_port: u16,

    /// SIP Realm / Domain (optional, defaults to server host)
    #[arg(long, global = true, env = "SIP_REALM")]
    realm: Option<String>,

    /// Asterisk AMI Port (Port 5038)
    #[arg(long, default_value_t = 5038, global = true, env = "ASTERISK_AMI_PORT")]
    ami_port: u16,

    /// Asterisk AMI Username
    #[arg(long, default_value = "admin", global = true, env = "ASTERISK_AMI_USER")]
    ami_user: String,

    /// Asterisk AMI Secret
    #[arg(long, default_value = "adminpass", global = true, env = "ASTERISK_AMI_SECRET")]
    ami_secret: String,

    /// Account username / extension
    #[arg(short, long, global = true, env = "PBX_USERNAME")]
    user: Option<String>,

    /// Account password / PIN
    #[arg(short, long, global = true, env = "PBX_PASSWORD")]
    password: Option<String>,

    /// Local IP address for SIP/RTP (defaults to auto-detection)
    #[arg(long, default_value = "auto", global = true)]
    local_ip: String,

    /// Local SIP UDP listening port
    #[arg(long, default_value_t = 50600, global = true)]
    local_sip_port: u16,

    /// Enable TLS for CDS / HTTPS
    #[arg(long, default_value_t = false, global = true)]
    tls: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Non-invasive probe: test server reachability, open ports and local IP without login
    Probe,

    /// Test authentication & SIP registration and print account details
    Login,

    /// Retrieve extensions, numbers, and account credentials
    Info,

    /// Fetch and display call journal / history
    Journal,

    /// View user presence (if supported by provider)
    Presence {
        #[arg(long)]
        status: Option<i32>,
        #[arg(short, long)]
        message: Option<String>,
    },

    /// Register to PBX and make an outbound call with live RTP audio
    Call {
        /// Phone number or extension to dial
        number: String,
        /// Call duration in seconds before automatically hanging up
        #[arg(short, long, default_value_t = 15)]
        duration: u64,
        /// DTMF digits to send during the call (e.g. "123#")
        #[arg(long)]
        dtmf: Option<String>,
        /// Perform hold and unhold test sequence during the call
        #[arg(long)]
        hold_test: bool,
        /// Blind transfer the call to an extension / number (SIP REFER RFC 3515)
        #[arg(long)]
        transfer_to: Option<String>,
    },

    /// Fetch and display contacts from the PBX directory / phonebook
    Phonebook,

    /// Run as background daemon with WebSocket IPC server for GTK4 or external frontends
    Serve {
        /// Port for the WebSocket IPC server
        #[arg(long, default_value_t = 8765)]
        ws_port: u16,
    },

    /// Run as background softphone daemon: keep SIP registered and listen for calls
    Listen,
}

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Auto-detect local IP if requested
    let local_ip = if cli.local_ip == "auto" {
        get_local_ip_for(&cli.server, cli.sip_port)
    } else {
        cli.local_ip.clone()
    };

    // Non-invasive probe does not require credentials
    if let Commands::Probe = cli.command {
        let provider: Box<dyn PbxProvider> = match cli.provider.to_lowercase().as_str() {
            "swyx" | "swyxware" => Box::new(
                SwyxProvider::new(&cli.server, "", "")
                    .with_ports(cli.cds_port, cli.sip_port, cli.tls)
                    .with_rest_port(cli.rest_port),
            ),
            "asterisk" => Box::new(
                AsteriskProvider::new(&cli.server, "", "")
                    .with_ami(cli.ami_port, &cli.ami_user, &cli.ami_secret),
            ),
            "sip" | "generic" | "fritzbox" | "freepbx" | "freeswitch" => {
                let mut p = GenericSipProvider::new(&cli.server, "", "").with_port(cli.sip_port);
                if let Some(r) = cli.realm {
                    p = p.with_realm(&r);
                }
                Box::new(p)
            }
            other => {
                eprintln!(
                    "Unknown provider '{}'. Supported: 'swyx', 'asterisk', 'sip' (FRITZ!Box, FreePBX).",
                    other
                );
                std::process::exit(1);
            }
        };

        println!("\n🔍 === {} Non-Invasive Network Probe ===", provider.name());
        println!("Target Server:       {}", cli.server);
        println!("Detected Local IP:   {}", local_ip);
        println!("--------------------------------------------------");
        println!("Checking ports without authentication...\n");

        let report = provider.probe(&local_ip).await?;
        for check in report.checks {
            let status = if check.open {
                "✅ OPEN / RESPONSIVE"
            } else {
                "❌ CLOSED / FILTERED"
            };
            let note = check
                .note
                .map(|n| format!(" ({})", n))
                .unwrap_or_default();
            println!(
                "  * Port {:<5} [{:<3}] {:<26} {}{}",
                check.port, check.protocol, check.service, status, note
            );
        }

        println!("--------------------------------------------------");
        println!("Probe complete. Zero credentials were sent.\n");
        return Ok(());
    }

    // Authenticated operations require username and password
    let user = cli.user.unwrap_or_else(|| {
        eprintln!("Error: Username is required for this command. Use -u <USER> or set PBX_USERNAME.");
        std::process::exit(1);
    });

    let password = cli.password.unwrap_or_else(|| {
        eprintln!("Error: Password is required for this command. Use -p <PASSWORD> or set PBX_PASSWORD.");
        std::process::exit(1);
    });

    let mut provider: Box<dyn PbxProvider> = match cli.provider.to_lowercase().as_str() {
        "swyx" | "swyxware" => Box::new(
            SwyxProvider::new(&cli.server, &user, &password)
                .with_ports(cli.cds_port, cli.sip_port, cli.tls)
                .with_rest_port(cli.rest_port),
        ),
        "asterisk" => Box::new(
            AsteriskProvider::new(&cli.server, &user, &password)
                .with_ami(cli.ami_port, &cli.ami_user, &cli.ami_secret),
        ),
        "sip" | "generic" | "fritzbox" | "freepbx" | "freeswitch" => {
            let mut p = GenericSipProvider::new(&cli.server, &user, &password).with_port(cli.sip_port);
            if let Some(r) = cli.realm {
                p = p.with_realm(&r);
            }
            Box::new(p)
        }
        other => {
            eprintln!(
                "Unknown provider '{}'. Supported: 'swyx', 'asterisk', 'sip' (FRITZ!Box, FreePBX).",
                other
            );
            std::process::exit(1);
        }
    };

    match cli.command {
        Commands::Probe => unreachable!(),

        Commands::Login => {
            info!("Connecting & registering with {}...", provider.name());
            provider.connect(&local_ip, cli.local_sip_port).await?;
            let info = provider.get_account_info().await?;
            println!("\n=== {} Login Successful ===", provider.name());
            println!("User ID:       {}", info.user_id);
            println!("SIP User:      {}", info.sip_user);
            println!("SIP Realm:     {}", info.sip_realm);
            if !info.extensions.is_empty() {
                println!("Extensions:    {}", info.extensions.join(", "));
            }
        }

        Commands::Info => {
            provider.connect(&local_ip, cli.local_sip_port).await?;
            let info = provider.get_account_info().await?;
            println!("\n=== {} Account Information ===", provider.name());
            println!("User ID:       {}", info.user_id);
            println!("SIP User:      {}", info.sip_user);
            println!("SIP Realm:     {}", info.sip_realm);
            if !info.extensions.is_empty() {
                println!("Extensions:    {}", info.extensions.join(", "));
            }
            if let Ok(Some(presence)) = provider.get_presence().await {
                println!("Status:        {:?} ({})", presence.status, presence.status_text);
            }
        }

        Commands::Journal => {
            provider.connect(&local_ip, cli.local_sip_port).await?;
            let journal = provider.get_call_journal().await?;
            if journal.is_empty() {
                println!("\n=== {} Call Journal ===", provider.name());
                println!("No entries found (or journal not supported by this provider).");
            } else {
                println!("\n=== {} Call Journal ({} entries) ===", provider.name(), journal.len());
                for entry in journal {
                    println!(
                        "[{}] {:<8} Caller: {:<15} Called: {:<15} Name: {:?}",
                        entry.timestamp,
                        format!("{:?}", entry.direction),
                        entry.caller_number,
                        entry.called_number,
                        entry.caller_name.unwrap_or_default()
                    );
                }
            }
        }

        Commands::Presence { .. } => {
            provider.connect(&local_ip, cli.local_sip_port).await?;
            match provider.get_presence().await? {
                Some(p) => {
                    println!("\n=== Rich Presence ===");
                    println!("User ID: {}", p.user_id);
                    println!("Status:  {:?}", p.status);
                    println!("Text:    {}", p.status_text);
                }
                None => {
                    println!("\nPresence is not supported by provider '{}'", provider.name());
                }
            }
        }

        Commands::Call {
            number,
            duration,
            dtmf,
            hold_test,
            transfer_to,
        } => {
            provider.connect(&local_ip, cli.local_sip_port).await?;
            println!("\nInitiating call to {} via {}...", number, provider.name());
            let mut call = provider.dial(&number).await?;
            println!("Call established! Call-ID: {}", call.call_id);

            if let Some(ref digits) = dtmf {
                println!("Sending DTMF sequence '{}'...", digits);
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                for c in digits.chars() {
                    let _ = provider.send_dtmf(&call, c).await;
                    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                }
            }

            if hold_test {
                println!("Streaming audio for 2 seconds before hold...");
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                println!("Putting call on HOLD (re-INVITE a=sendonly)...");
                provider.hold(&mut call).await?;
                println!("Call is now ON HOLD. Waiting 3 seconds...");
                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

                println!("UNHOLDING call (re-INVITE a=sendrecv)...");
                provider.unhold(&mut call).await?;
                println!("Call RESUMED from hold!");
            }

            if let Some(target) = transfer_to {
                println!("Transferring call to {} via SIP REFER (RFC 3515)...", target);
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                provider.transfer(&mut call, &target).await?;
                println!("Call successfully transferred to {}! Exiting.", target);
                return Ok(());
            }

            println!("Streaming audio for {} seconds...", duration);
            tokio::time::sleep(tokio::time::Duration::from_secs(duration)).await;

            println!("Terminating call...");
            provider.hangup(&call).await?;
            println!("Call ended.");
        }

        Commands::Phonebook => {
            provider.connect(&local_ip, cli.local_sip_port).await?;
            let contacts = provider.get_phonebook().await?;
            println!(
                "\n=== {} Phonebook / Contacts ({} entries) ===",
                provider.name(),
                contacts.len()
            );
            for c in contacts {
                println!(
                    "  * {:<25} Number: {:<12} Ext: {:<8} Dept: {:?}",
                    c.name,
                    c.number,
                    c.extension.unwrap_or_default(),
                    c.department.unwrap_or_default()
                );
            }
        }

        Commands::Serve { ws_port } => {
            let engine = deskphone_core::SoftphoneEngine::new(provider);
            engine.start(&local_ip, cli.local_sip_port).await?;

            let addr: std::net::SocketAddr = format!("0.0.0.0:{}", ws_port).parse()?;
            println!(
                "\n🚀 Softphone Daemon started with WebSocket IPC on ws://{}",
                addr
            );
            println!("Ready for GTK4 or external UI connections. Press Ctrl+C to terminate.");

            deskphone_core::run_websocket_server(engine, addr).await?;
        }

        Commands::Listen => {
            provider.connect(&local_ip, cli.local_sip_port).await?;
            println!(
                "\nSoftphone active and registered on {}! Press Ctrl+C to stop.",
                provider.name()
            );
            tokio::signal::ctrl_c().await?;
            println!("Shutting down softphone...");
        }
    }

    Ok(())
}
