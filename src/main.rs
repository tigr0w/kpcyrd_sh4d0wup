use clap::Parser;
use env_logger::Env;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use openssl::hash::MessageDigest;
use sh4d0wup::args::{self, Args, Hsm, HsmPgp, Infect, Keygen, Sign, SubCommand, Tamper};
use sh4d0wup::build;
use sh4d0wup::check;
use sh4d0wup::errors::*;
use sh4d0wup::hsm;
use sh4d0wup::httpd;
use sh4d0wup::infect;
use sh4d0wup::keygen::in_toto::InTotoEmbedded;
use sh4d0wup::keygen::openssl::OpensslEmbedded;
use sh4d0wup::keygen::pgp::PgpEmbedded;
use sh4d0wup::keygen::{self, EmbeddedKey};
use sh4d0wup::plot::{PatchAptReleaseConfig, PatchPkgDatabaseConfig, PlotExtras};
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
            let ctx = bait.plot.load_into_context().await?;

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
            } else if let Some(tls) = ctx.plot.tls.clone() {
                Some(httpd::Tls::try_from(tls)?)
            } else {
                None
            };

            if !bait.no_bind {
                httpd::run(bait.bind, tls, ctx).await?;
            }
        }
        SubCommand::Infect(Infect::Pacman(infect)) => {
            let pkg = fs::read(&infect.path)?;
            let mut out = File::create(&infect.out)?;
            infect::pacman::infect(&infect.try_into()?, &pkg, &mut out)?;
        }
        SubCommand::Infect(Infect::Deb(infect)) => {
            let pkg = fs::read(&infect.path)?;
            let mut out = File::create(&infect.out)?;
            infect::deb::infect(&infect.try_into()?, &pkg, &mut out)?;
        }
        SubCommand::Infect(Infect::Oci(infect)) => {
            let pkg = fs::read(&infect.path)?;
            let mut out = File::create(&infect.out)?;
            infect::oci::infect(&infect, &pkg, &mut out)?;
        }
        SubCommand::Infect(Infect::Apk(infect)) => {
            let pkg = fs::read(&infect.path)?;
            let signing_key = OpensslEmbedded::read_from_disk(&infect.signing_key)?;

            let mut out = File::create(&infect.out)?;

            let mut plot_extras = PlotExtras::default();
            plot_extras
                .signing_keys
                .insert("openssl".to_string(), EmbeddedKey::Openssl(signing_key));

            infect::apk::infect(
                &infect::apk::Infect {
                    signing_key: "openssl".to_string(),
                    signing_key_name: infect.signing_key_name,
                    payload: infect.payload,
                },
                &plot_extras.signing_keys,
                &pkg,
                &mut out,
            )?;
        }
        SubCommand::Infect(Infect::Elf(infect)) => {
            let elf = fs::read(&infect.path)?;
            let mut out = tokio::fs::File::create(&infect.out).await?;
            infect::elf::infect(&infect.try_into()?, &elf, &mut out).await?;
        }
        SubCommand::Tamper(Tamper::PacmanDb(tamper)) => {
            let db = fs::read(&tamper.path)?;
            let mut out = File::create(&tamper.out)?;

            let config = PatchPkgDatabaseConfig::<Vec<String>>::from_args(tamper.config)?;
            let plot_extras = PlotExtras::default();
            tamper::pacman::patch(&config, &plot_extras, &db, &mut out)?;
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

            let mut plot_extras = PlotExtras::default();
            let signing_key = if let Some(signing_key) = signing_key {
                plot_extras
                    .signing_keys
                    .insert("pgp".to_string(), EmbeddedKey::Pgp(signing_key));
                Some("pgp".to_string())
            } else {
                None
            };
            let config = PatchAptReleaseConfig {
                fields: release_fields,
                checksums: checksum_config,
                signing_key,
            };
            tamper::apt_release::patch(
                &config,
                &plot_extras.artifacts,
                &plot_extras.signing_keys,
                &db,
                &mut out,
            )?;
        }
        SubCommand::Tamper(Tamper::AptPackageList(tamper)) => {
            let db = fs::read(&tamper.path)?;
            let mut out = File::create(&tamper.out)?;

            let plot_extras = PlotExtras::default();
            let config = PatchPkgDatabaseConfig::<Vec<String>>::from_args(tamper.config)?;
            tamper::apt_package_list::patch(&config, None, &plot_extras.artifacts, &db, &mut out)?;
        }
        SubCommand::Tamper(Tamper::ApkIndex(tamper)) => {
            let db = fs::read(&tamper.path)?;
            let mut out = File::create(&tamper.out)?;

            let signing_key = OpensslEmbedded::read_from_disk(tamper.signing_key)?;

            let mut plot_extras = PlotExtras::default();
            plot_extras
                .signing_keys
                .insert("openssl".to_string(), EmbeddedKey::Openssl(signing_key));

            let config = PatchPkgDatabaseConfig::<String>::from_args(tamper.config)?;
            tamper::apk::patch(
                &config,
                &plot_extras,
                "openssl",
                &tamper.signing_key_name,
                &db,
                &mut out,
            )?;
        }
        SubCommand::Check(check) => {
            let ctx = check.plot.load_into_context().await?;
            match check.no_exec {
                0 => {
                    check::spawn(check, ctx).await?;
                }
                1 => {
                    serde_json::to_writer_pretty(io::stdout(), &ctx.plot)?;
                    println!();
                }
                _ => {
                    serde_json::to_writer_pretty(io::stdout(), &ctx.plot)?;
                    println!();
                    for (key, value) in &ctx.extras.signing_keys {
                        println!("signing_key {:?}: {:?}", key, value);
                    }
                    for (key, value) in &ctx.extras.artifacts {
                        println!("artifact {:?}: {} bytes", key, value.len());
                    }
                }
            }
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
            keygen::pgp::debug_inspect(pgp.secret_key.as_bytes())
                .context("Failed to inspect serialized pgp data")?;
            print!("{}", pgp.secret_key);
            if let Some(rev) = pgp.rev {
                keygen::pgp::debug_inspect(rev.as_bytes())
                    .context("Failed to inspect serialized pgp data")?;
                print!("{}", rev);
            }
        }
        SubCommand::Keygen(Keygen::Ssh(ssh)) => {
            let ssh =
                keygen::ssh::generate(&ssh.try_into()?).context("Failed to generate ssh key")?;
            if let Some(public_key) = ssh.public_key {
                println!("{}", public_key);
            }
            print!("{}", ssh.secret_key);
        }
        SubCommand::Keygen(Keygen::Openssl(openssl)) => {
            let openssl = keygen::openssl::generate(&openssl.try_into()?)
                .context("Failed to generate openssl key")?;
            if let Some(public_key) = openssl.public_key {
                print!("{}", public_key);
            }
            print!("{}", openssl.secret_key);
        }
        SubCommand::Keygen(Keygen::InToto(in_toto)) => {
            let in_toto = keygen::in_toto::generate(&in_toto.into())
                .context("Failed to generate in-toto key")?;
            if let Some(public_key) = in_toto.public_key {
                println!("{}", public_key);
            }
            println!("{}", in_toto.secret_key);
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
        SubCommand::Sign(Sign::Openssl(openssl)) => {
            if !openssl.binary {
                bail!("Openssl signatures are always binary");
            }
            let secret_key = OpensslEmbedded::read_from_disk(&openssl.secret_key)
                .context("Failed to load secret key")?;
            debug!("Reading data to sign...");
            let data = fs::read(&openssl.path)
                .with_context(|| anyhow!("Failed to read payload data from {:?}", openssl.path))?;

            let digest = if openssl.md5 {
                MessageDigest::md5()
            } else if openssl.sha1 {
                MessageDigest::sha1()
            } else if openssl.sha256 {
                MessageDigest::sha256()
            } else if openssl.sha512 {
                MessageDigest::sha512()
            } else if openssl.sha3_256 {
                MessageDigest::sha3_256()
            } else if openssl.sha3_512 {
                MessageDigest::sha3_512()
            } else {
                bail!("No hash function selected")
            };

            let sig = sign::openssl::sign(&secret_key, &data, digest)?;
            io::stdout().write_all(&sig)?;
        }
        SubCommand::Sign(Sign::InToto(in_toto)) => {
            let secret_key = InTotoEmbedded::read_from_disk(&in_toto.secret_key)
                .context("Failed to load secret key")?;
            let mut sig = sign::in_toto::sign(&secret_key, &in_toto.try_into()?)?;
            sig.push(b'\n');
            io::stdout().write_all(&sig)?;
        }
        SubCommand::Hsm(Hsm::Pgp(HsmPgp::Access(access))) => hsm::pgp::access(&access)?,
        SubCommand::Build(build) => build::run(build).await?,
        SubCommand::Req(req) => {
            let ctx = req.plot.load_into_context().await?;

            let mut headers = HeaderMap::new();
            for header in &req.headers {
                let (k, v) = header.split_once(':').with_context(|| {
                    anyhow!("Configured header has no value (missing `:`): {:?}", header)
                })?;
                let v = v.strip_prefix(' ').unwrap_or(v);
                headers.insert(
                    HeaderName::from_bytes(k.as_bytes())?,
                    HeaderValue::try_from(v)?,
                );
            }

            let route_action = ctx
                .plot
                .select_route(&req.req_path, req.addr.as_ref(), &headers)
                .context("Failed to select route")?;
            info!("Selected route: {:?}", route_action);
        }
        SubCommand::Completions(completions) => {
            args::gen_completions(&completions)?;
        }
    }

    Ok(())
}
