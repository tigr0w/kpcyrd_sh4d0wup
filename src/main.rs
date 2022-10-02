use clap::Parser;
use env_logger::Env;
use sh4d0wup::args::{Args, Infect, SubCommand, TamperIdx};
use sh4d0wup::errors::*;
use sh4d0wup::httpd;
use sh4d0wup::infect;
use sh4d0wup::plot::Plot;
use sh4d0wup::tamper_idx;
use std::fs;
use std::fs::File;

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
            httpd::run(bait.bind, plot).await?;
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
        SubCommand::TamperIdx(TamperIdx::Pacman(tamper_idx)) => {
            let db = fs::read(&tamper_idx.path)?;
            let mut out = File::create(&tamper_idx.out)?;

            let config = tamper_idx::pacman::PacmanPatchConfig::from_args(tamper_idx)?;
            tamper_idx::pacman::patch_database(&config, &db, &mut out)?;
        }
    }

    Ok(())
}
