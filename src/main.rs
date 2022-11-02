use clap::Parser;
use env_logger::Env;
use sh4d0wup::args::{self, Args, Infect, Keygen, Sign, SubCommand, Tamper};
use sh4d0wup::check;
use sh4d0wup::errors::*;
use sh4d0wup::httpd;
use sh4d0wup::infect;
use sh4d0wup::keygen;
use sh4d0wup::keygen::pgp::PgpEmbedded;
use sh4d0wup::plot::{PatchAptReleaseConfig, PatchPkgDatabaseConfig, Plot};
use sh4d0wup::sign;
use sh4d0wup::tamper;
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let log_level = match args.verbose {
        0 => "sh4d0wup=info",
        1 => "info,sh4d0wup=debug",
        2 => "debug",
        3 => "debug,sh4d0wup=trace",
        _ => "trace",
    };
    env_logger::init_from_env(Env::default().default_filter_or(log_level));

    match args.subcommand {
        SubCommand::Bait(bait) => {
            info!("Loading plot from {:?}...", bait.plot);
            let plot = Plot::load_from_path(&bait.plot)?;
            trace!("Loaded plot: {:?}", plot);
            let tls = if let Some(path) = bait.tls_cert {
                let cert = fs::read(&path)
                    .with_context(|| anyhow!("Failed to read certificate from path: {:?}", path))?;

                let key = if let Some(path) = bait.tls_key {
                    fs::read(&path).with_context(|| {
                        anyhow!("Failed to read certificate from path: {:?}", path)
                    })?
                } else {
                    cert.clone()
                };

                Some(httpd::Tls { cert, key })
            } else if let Some(tls) = plot.tls.clone() {
                Some(httpd::Tls::try_from(tls)?)
            } else {
                None
            };

            httpd::run(bait.bind, tls, plot).await?;
        }
        SubCommand::Infect(Infect::Pacman(infect)) => {
            let pkg = fs::read(&infect.path)?;
            let mut out = File::create(&infect.out)?;
            infect::pacman::infect(&infect, &pkg, &mut out)?;
        }
        SubCommand::Infect(Infect::Deb(infect)) => {
            let pkg = fs::read(&infect.path)?;
            let mut out = File::create(&infect.out)?;
            infect::deb::infect(&infect, &pkg, &mut out)?;
        }
        SubCommand::Infect(Infect::Oci(infect)) => {
            let pkg = fs::read(&infect.path)?;
            let mut out = File::create(&infect.out)?;
            infect::oci::infect(&infect, &pkg, &mut out)?;
        }
        SubCommand::Infect(Infect::Apk(infect)) => {
            let pkg = fs::read(&infect.path)?;
            let mut out = File::create(&infect.out)?;
            infect::apk::infect(&infect, &pkg, &mut out)?;
        }
        SubCommand::Infect(Infect::Elf(infect)) => {
            let elf = fs::read(&infect.path)?;
            infect::elf::infect(&infect, &elf, &infect.out).await?;
        }
        SubCommand::Tamper(Tamper::PacmanDb(tamper)) => {
            let db = fs::read(&tamper.path)?;
            let mut out = File::create(&tamper.out)?;

            let config = PatchPkgDatabaseConfig::<Vec<String>>::from_args(tamper.config)?;
            tamper::pacman::patch_database(&config, &db, &mut out)?;
        }
        SubCommand::Tamper(Tamper::AptRelease(tamper)) => {
            let db = fs::read(&tamper.path)?;
            let mut out = File::create(&tamper.out)?;

            let checksum_config = PatchPkgDatabaseConfig::<String>::from_args(tamper.config)?;

            let mut release_fields = BTreeMap::new();

            for s in &tamper.release_set {
                let (key, value) = s.split_once('=').context("Argument is not an assignment")?;
                release_fields.insert(key.to_string(), value.to_string());
            }

            let signing_key = match (tamper.unsigned, tamper.signing_key) {
                (true, Some(_)) => {
                    warn!("Using --unsigned and --signing-key together is causing the release to be unsigned");
                    None
                }
                (true, None) => None,
                (false, Some(path)) => Some(PgpEmbedded::read_from_disk(path)?),
                (false, None) => bail!("Missing --signing-key and --unsigned wasn't provided"),
            };

            let mut keys = BTreeMap::new();
            let signing_key = if let Some(signing_key) = signing_key {
                keys.insert("pgp".to_string(), signing_key);
                Some("pgp".to_string())
            } else {
                None
            };

            let config = PatchAptReleaseConfig {
                fields: release_fields,
                checksums: checksum_config,
                signing_key,
            };
            tamper::apt_release::patch(&config, &keys, &db, &mut out)?;
        }
        SubCommand::Tamper(Tamper::AptPackageList(tamper)) => {
            let db = fs::read(&tamper.path)?;
            let mut out = File::create(&tamper.out)?;

            let config = PatchPkgDatabaseConfig::<Vec<String>>::from_args(tamper.config)?;
            tamper::apt_package_list::patch(&config, &db, &mut out)?;
        }
        SubCommand::Check(check) => {
            info!("Loading plot from {:?}...", check.plot);
            let plot = Plot::load_from_path(&check.plot)?;
            trace!("Loaded plot: {:?}", plot);
            check::spawn(check, plot).await?;
        }
        SubCommand::Keygen(Keygen::Tls(tls)) => {
            let tls =
                keygen::tls::generate(tls.into()).context("Failed to generate tls certificate")?;
            print!("{}", tls.cert);
            print!("{}", tls.key);
        }
        SubCommand::Keygen(Keygen::Pgp(pgp)) => {
            let pgp = keygen::pgp::generate(pgp.into()).context("Failed to generate pgp key")?;
            if let Some(cert) = pgp.cert {
                keygen::pgp::debug_inspect(cert.as_bytes())
                    .context("Failed to inspect serialized pgp data")?;
                print!("{}", cert);
            }
            keygen::pgp::debug_inspect(pgp.key.as_bytes())
                .context("Failed to inspect serialized pgp data")?;
            print!("{}", pgp.key);
            if let Some(rev) = pgp.rev {
                keygen::pgp::debug_inspect(rev.as_bytes())
                    .context("Failed to inspect serialized pgp data")?;
                print!("{}", rev);
            }
        }
        SubCommand::Sign(Sign::PgpCleartext(pgp)) => {
            if pgp.binary {
                bail!("Binary output is not supported for cleartext signatures");
            }
            let secret_key = PgpEmbedded::read_from_disk(&pgp.secret_key)
                .context("Failed to load secret key")?;
            debug!("Reading data to sign...");
            let data = fs::read(&pgp.path)
                .with_context(|| anyhow!("Failed to read payload data from {:?}", pgp.path))?;
            let sig = sign::pgp::sign_cleartext(&secret_key, &data)?;
            io::stdout().write_all(&sig)?;
        }
        SubCommand::Sign(Sign::PgpDetached(pgp)) => {
            let secret_key = PgpEmbedded::read_from_disk(&pgp.secret_key)
                .context("Failed to load secret key")?;
            debug!("Reading data to sign...");
            let data = fs::read(&pgp.path)
                .with_context(|| anyhow!("Failed to read payload data from {:?}", pgp.path))?;
            let sig = sign::pgp::sign_detached(&secret_key, &data, pgp.binary)?;
            io::stdout().write_all(&sig)?;
        }
        SubCommand::Completions(completions) => {
            args::gen_completions(&completions)?;
        }
    }

    Ok(())
}
