use crate::codegen;
use crate::errors::*;
use crate::keygen;
use crate::plot;
use crate::plot::{PkgFilter, PkgPatchValues};
use crate::sign::in_toto::VirtualEntry;
use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use std::io::stdout;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::PathBuf;
use url::Url;

#[derive(Debug, Parser)]
#[command(version)]
pub struct Args {
    /// Increase logging output (can be used multiple times)
    #[arg(short, long, global = true, action(ArgAction::Count))]
    pub verbose: u8,
    /// Reduce logging output (can be used multiple times)
    #[arg(short, long, global = true, action(ArgAction::Count))]
    pub quiet: u8,
    #[command(subcommand)]
    pub subcommand: SubCommand,
}

#[derive(Debug, Subcommand)]
pub enum SubCommand {
    Bait(Bait),
    Front(Front),
    #[command(subcommand)]
    Infect(Infect),
    #[command(subcommand)]
    Tamper(Tamper),
    #[command(subcommand)]
    Keygen(Keygen),
    #[command(subcommand)]
    Sign(Sign),
    #[command(subcommand)]
    Hsm(Hsm),
    Build(Build),
    Check(Check),
    Req(Req),
    Completions(Completions),
}

#[derive(Debug, Clone, Parser)]
pub struct Plot {
    /// Path to the plot to execute
    #[arg(value_name = "PLOT-PATH")]
    pub path: PathBuf,
    /// Use artifacts from compiled plot as download cache
    #[arg(long = "cache-from")]
    pub cache_from: Option<PathBuf>,
}

impl Plot {
    pub async fn load_into_context(&self) -> Result<plot::Ctx> {
        info!("Loading plot from {:?}...", self.path);
        plot::Ctx::load_from_path(&self.path, self.cache_from.as_deref()).await
    }
}

#[derive(Debug, Clone, Parser)]
pub struct Bind {
    /// Address to bind to
    #[arg(
        short = 'B',
        long,
        env = "SH4D0WUP_BIND",
        default_value = "0.0.0.0:1337"
    )]
    pub addr: SocketAddr,
    /// Setup the attack but exit instead of serving requests
    #[arg(short, long)]
    pub no_bind: bool,
}

#[derive(Debug, Clone, Parser)]
pub struct Tls {
    /// Path to certificate file (enables https)
    #[arg(long)]
    pub cert: Option<PathBuf>,
    /// Path to certificate private key (if not bundled with the cert)
    #[arg(long)]
    pub key: Option<PathBuf>,
}

/// Start a malicious update server
#[derive(Debug, Clone, Parser)]
pub struct Bait {
    #[clap(flatten)]
    pub plot: Plot,
    #[clap(flatten)]
    pub bind: Bind,
    #[clap(flatten)]
    pub tls: Tls,
}

/// Bind a http/https server but forward everything unmodified
#[derive(Debug, Clone, Parser)]
pub struct Front {
    /// The upstream server to reverse proxy to
    #[arg(short = 'U', long)]
    pub upstream: Url,
    #[clap(flatten)]
    pub bind: Bind,
    #[clap(flatten)]
    pub tls: Tls,
}

/// High level tampering, inject additional commands into a package
#[derive(Debug, Subcommand)]
pub enum Infect {
    Pacman(InfectPacmanPkg),
    Deb(InfectDebPkg),
    Oci(InfectOci),
    Apk(InfectApkPkg),
    Elf(InfectElf),
    ElfFwdStdin(InfectElfFwdStdin),
    Sh(InfectSh),
}

/// Infect a pacman package
#[derive(Debug, Clone, Parser)]
pub struct InfectPacmanPkg {
    /// The input package to use as a base
    pub path: PathBuf,
    /// Where to write the modified package to
    pub out: PathBuf,
    /// Update a key in .PKGINFO (a key can be set multiple times)
    #[arg(long)]
    pub set: Vec<String>,
    /// The command to inject into the package that's executed once during install
    #[arg(short = 'c', long)]
    pub payload: Option<String>,
}

/// Infect a .deb package
#[derive(Debug, Clone, Parser)]
pub struct InfectDebPkg {
    /// The input package to use as a base
    pub path: PathBuf,
    /// Where to write the modified package to
    pub out: PathBuf,
    /// Update a key in ./control
    #[arg(long)]
    pub set: Vec<String>,
    /// The command to inject into the package that's executed once during install
    #[arg(short = 'c', long)]
    pub payload: Option<String>,
}

/// Infect an OCI container image .tar
#[derive(Debug, Clone, Parser)]
pub struct InfectOci {
    /// The input package to use as a base
    pub path: PathBuf,
    /// Where to write the modified package to
    pub out: PathBuf,
    /// Configure the length of the entrypoint hash, longer names are less likely to collide but more noticable
    #[arg(long, default_value = "14")]
    pub entrypoint_hash_len: usize,
    /// Write the entrypoint to a fixed location instead of a random one
    #[arg(long)]
    pub entrypoint: Option<String>,
    /// The command to inject into the package that's executed once during install
    #[arg(short = 'c', long)]
    pub payload: Option<String>,
    /// Change the tags of the modified images
    #[arg(short = 't', long = "tag")]
    pub tags: Vec<String>,
}

/// Infect an alpine apk package
#[derive(Debug, Clone, Parser)]
pub struct InfectApkPkg {
    /// The input package to use as a base
    pub path: PathBuf,
    /// Where to write the modified package to
    pub out: PathBuf,
    /// Update a key in .PKGINFO (a key can be set multiple times)
    #[arg(long)]
    pub set: Vec<String>,
    /// Path to the key to sign the package with
    #[arg(short = 'S', long)]
    pub signing_key: PathBuf,
    /// The name of the signing key (eg. alpine-devel@lists.alpinelinux.org-6165ee59.rsa.pub)
    #[arg(short = 'N', long)]
    pub signing_key_name: String,
    /// The command to inject into the package that's executed once during install
    #[arg(short = 'c', long)]
    pub payload: Option<String>,
}

#[derive(Debug, Clone, Parser)]
pub struct CompileElfConfig {
    /// The codegen backend that should be used to compile a binary
    #[arg(short = 'B', long)]
    pub backend: Option<codegen::Backend>,
    /// Specify the target architecture, not supported by all backends yet
    #[arg(short, long)]
    pub target: Option<String>,
}

/// Infect an elf executable
#[derive(Debug, Clone, Parser)]
pub struct InfectElf {
    /// The input executable to bind to
    pub path: PathBuf,
    /// Where to write the modified executable to
    pub out: PathBuf,
    #[clap(flatten)]
    pub compile: CompileElfConfig,
    /// The command to inject into the binary
    #[arg(short = 'c', long)]
    pub payload: Option<String>,
    /// Spawn an embedded elf binary instead of executing a shell command
    #[arg(short = 'e', long)]
    pub elf: Option<String>,
    /// Instead of unpacking to memory, replace the binary on disk with the input executable
    #[arg(long)]
    pub self_replace: bool,
    /// Instead of detecting the local path with readlink(/proc/self/exe), hard-code a value
    #[arg(long)]
    pub assume_path: Option<String>,
}

/// Convert a shell script into an elf
#[derive(Debug, Clone, Parser)]
pub struct InfectElfFwdStdin {
    /// The input data that should be sent to the child's stdin at runtime
    pub path: PathBuf,
    /// Where to write the generated elf binary to
    pub out: PathBuf,
    /// The binary that should be executed, defaults to /bin/sh
    #[arg(long)]
    pub exec: Option<String>,
    /// Additional arguments that should be passed to the binary. Can be used multiple times and `argv[0]` needs to be provided, if none are set then `argv[0]` defaults to the value of `--exec`
    #[arg(long = "arg")]
    pub args: Vec<String>,
    #[clap(flatten)]
    pub compile: CompileElfConfig,
}

/// Infect a shell script
#[derive(Debug, Clone, Parser)]
pub struct InfectSh {
    /// The input script to infect
    pub path: PathBuf,
    /// Where to write the infected script to
    pub out: PathBuf,
    /// The command to inject into the binary
    #[arg(short = 'c', long)]
    pub payload: String,
    /// Names of functions our payload should be hooked into (it only executes once)
    #[arg(long = "hook")]
    pub hooks: Vec<String>,
}

/// Low level tampering, patch a package database to add malicious packages, cause updates or influence dependency resolution
#[derive(Debug, Subcommand)]
pub enum Tamper {
    PacmanDb(TamperPacman),
    AptRelease(TamperAptRelease),
    AptPackageList(TamperAptPackageList),
    ApkIndex(TamperApkIndex),
    GitCommit(TamperGitCommit),
}

#[derive(Debug, Clone, Parser)]
pub struct TamperPackageDatabaseConfig {
    #[arg(long)]
    pub filter: Vec<PkgFilter>,
    #[arg(long)]
    pub set: Vec<PkgPatchValues<Vec<String>>>,
    #[arg(long)]
    pub exclude: Vec<PkgFilter>,
}

/// Patch a pacman database
#[derive(Debug, Clone, Parser)]
pub struct TamperPacman {
    /// The input database to modify
    pub path: PathBuf,
    /// Path to write the patched database to
    pub out: PathBuf,
    #[clap(flatten)]
    pub config: TamperPackageDatabaseConfig,
}

/// Patch an apt `InRelease` file
#[derive(Debug, Clone, Parser)]
pub struct TamperAptRelease {
    /// The input database to modify
    pub path: PathBuf,
    /// Path to write the patched database to
    pub out: PathBuf,
    /// Patch a metadata field on the release instead of a checksum
    #[arg(long)]
    pub release_set: Vec<String>,
    /// Skip signing the final release file
    #[arg(long)]
    pub unsigned: bool,
    /// Path to signing key to sign the release with
    #[arg(long)]
    pub signing_key: Option<PathBuf>,
    #[clap(flatten)]
    pub config: TamperPackageDatabaseConfig,
}

/// Patch an apt `Packages` file
#[derive(Debug, Clone, Parser)]
pub struct TamperAptPackageList {
    /// The input database to modify
    pub path: PathBuf,
    /// Path to write the patched database to
    pub out: PathBuf,
    #[clap(flatten)]
    pub config: TamperPackageDatabaseConfig,
}

/// Patch an alpine apk `APKINDEX.tar.gz` file
#[derive(Debug, Clone, Parser)]
pub struct TamperApkIndex {
    /// The input database to modify
    pub path: PathBuf,
    /// Path to write the patched database to
    pub out: PathBuf,
    #[clap(flatten)]
    pub config: TamperPackageDatabaseConfig,
    /// Path to the key to sign the package with
    #[arg(short = 'S', long)]
    pub signing_key: PathBuf,
    /// The name of the signing key (eg. alpine-devel@lists.alpinelinux.org-6165ee59.rsa.pub)
    #[arg(short = 'N', long)]
    pub signing_key_name: String,
}

/// Generate a git commit from the given parameters (optionally bruteforce a hash prefix)
#[derive(Debug, Clone, Parser)]
pub struct TamperGitCommit {
    /// The hash of the tree object this commit should point to
    #[arg(long)]
    pub tree: Option<String>,
    /// The parent commit of this commit (can be multiple)
    #[arg(long = "parent")]
    pub parents: Vec<String>,
    /// Remove any referenced parent commits from the commit
    #[arg(long)]
    pub no_parents: bool,
    /// The author of the commit in the format `John Doe <git@example.com> 1637076383 +0100`
    #[arg(long)]
    pub author: Option<String>,
    /// The committer of the commit in the format `John Doe <git@example.com> 1637076383 +0100`
    #[arg(long)]
    pub committer: Option<String>,
    /// The commit message (if omitted, read from stdin)
    #[arg(long)]
    pub message: Option<String>,
    /// Add a nonce value to the commit and increment it until the commit object hash matches the prefix
    #[arg(long)]
    pub collision_prefix: Option<String>,
    /// Add an additional value to the commit to influence the hash
    #[arg(long)]
    pub nonce: Option<String>,
    /// Read a git commit object from stdin and use it as a base
    #[arg(long)]
    pub stdin: bool,
    /// Read a commit message from stdin
    #[arg(long, conflicts_with_all=&["stdin", "message"])]
    pub message_stdin: bool,
    /// Remove the loose header to convert it into the format expected by `git hash-object -t commit -w --stdin`
    #[arg(long)]
    pub strip_header: bool,
}

/// Generate signing keys with the given parameters
#[derive(Debug, Subcommand)]
pub enum Keygen {
    Tls(KeygenTls),
    Pgp(KeygenPgp),
    Ssh(KeygenSsh),
    Openssl(KeygenOpenssl),
    InToto(KeygenInToto),
}

#[derive(Debug, Clone, Parser)]
pub struct KeygenFlags {
    /// Only print the generated private key
    #[arg(short = 'S', long)]
    pub secret_key_only: bool,
    /// Only print the generated public key
    #[arg(short = 'P', long)]
    pub public_key_only: bool,
}

impl KeygenFlags {
    pub fn all(&self) -> bool {
        !self.secret_key_only && !self.public_key_only
    }

    pub fn secret_key(&self) -> bool {
        self.all() || self.secret_key_only
    }

    pub fn public_key(&self) -> bool {
        self.all() || self.public_key_only
    }
}

/// Generate a self-signed tls certificate
#[derive(Debug, Clone, Parser)]
pub struct KeygenTls {
    pub names: Vec<String>,
    /// Generate a PKCS_ECDSA_P256_SHA256 keypair (default)
    #[arg(long, group = "algo")]
    pub ecdsa: bool,
    /// Generate a PKCS_ED25519 keypair
    #[arg(long, group = "algo")]
    pub ed25519: bool,
    /// Shorthand for --rsa-sha256
    #[arg(long, group = "algo")]
    pub rsa: bool,
    /// Generate a PKCS_RSA_SHA256 keypair
    #[arg(long, group = "algo")]
    pub rsa_sha256: bool,
    /// Generate a PKCS_RSA_SHA512 keypair
    #[arg(long, group = "algo")]
    pub rsa_sha512: bool,
    #[command(flatten)]
    pub flags: KeygenFlags,
}

/// Generate a pgp keypair
#[derive(Debug, Clone, Parser)]
pub struct KeygenPgp {
    pub uids: Vec<String>,
    #[command(flatten)]
    pub flags: KeygenFlags,
}

/// Generate an ssh keypair
#[derive(Debug, Clone, Parser)]
pub struct KeygenSsh {
    /// The type of key to generate
    #[arg(short = 't', long = "type", default_value = "ed25519")]
    pub keytype: keygen::ssh::KeypairType,
    /// Number of bits to use for the keypair
    #[arg(short, long)]
    pub bits: Option<usize>,
    #[command(flatten)]
    pub flags: KeygenFlags,
}

/// Generate an openssl keypair
#[derive(Debug, Clone, Parser)]
pub struct KeygenOpenssl {
    /// Generate an rsa keypair
    #[arg(long, group = "keypair")]
    pub rsa: bool,
    /// Generate an secp256k1 keypair
    #[arg(long, group = "keypair")]
    pub secp256k1: bool,
    /// Number of bits to use for the keypair
    #[arg(short, long)]
    pub bits: Option<u32>,
    #[command(flatten)]
    pub flags: KeygenFlags,
}

/// Generate an in-toto keypair
#[derive(Debug, Clone, Parser)]
pub struct KeygenInToto {
    #[command(flatten)]
    pub flags: KeygenFlags,
}

/// Use signing keys to generate signatures
#[derive(Debug, Subcommand)]
pub enum Sign {
    /// Create a cleartext pgp signature
    PgpCleartext(SignPgp),
    /// Create a detached pgp signature
    PgpDetached(SignPgp),
    Openssl(SignOpenssl),
    InToto(SignInToto),
}

#[derive(Debug, Clone, Parser)]
pub struct SignPgp {
    /// Secret key to use for signing
    #[arg(short, long)]
    pub secret_key: PathBuf,
    /// Don't use ascii armor
    #[arg(short, long)]
    pub binary: bool,
    /// Path to data to sign
    pub path: PathBuf,
}

/// Create a digest signature with openssl
#[derive(Debug, Clone, Parser)]
pub struct SignOpenssl {
    /// Secret key to use for signing
    #[arg(short, long)]
    pub secret_key: PathBuf,
    /// Don't use ascii armor
    #[arg(short, long)]
    pub binary: bool,
    /// Path to data to sign
    pub path: PathBuf,
    /// Use md5 hash function
    #[arg(long, group = "hash")]
    pub md5: bool,
    /// Use sha1 hash function
    #[arg(long, group = "hash")]
    pub sha1: bool,
    /// Use sha256 hash function
    #[arg(long, group = "hash")]
    pub sha256: bool,
    /// Use sha512 hash function
    #[arg(long, group = "hash")]
    pub sha512: bool,
    /// Use sha3-256 hash function
    #[arg(long, group = "hash")]
    pub sha3_256: bool,
    /// Use sha3-512 hash function
    #[arg(long, group = "hash")]
    pub sha3_512: bool,
}

/// Create an in-toto attestation
#[derive(Debug, Clone, Parser)]
pub struct SignInToto {
    /// Secret key to use for signing
    #[arg(short, long)]
    pub secret_key: PathBuf,
    /// Path to data to sign
    pub path: PathBuf,
    /// Identify attestation
    #[arg(long)]
    pub name: String,
    /// Build input for the attestation (format: name=/path/to/file.txt)
    #[arg(long)]
    pub material: Vec<VirtualEntry>,
    /// Build output for the attestation (format: name=/path/to/file.txt)
    #[arg(long)]
    pub product: Vec<VirtualEntry>,
}

/// Interact with hardware signing keys
#[derive(Debug, Subcommand)]
pub enum Hsm {
    #[clap(subcommand)]
    Pgp(HsmPgp),
}

/// Interact with pgp hardware signing keys
#[derive(Debug, Subcommand)]
pub enum HsmPgp {
    Access(HsmAccess),
}

/// Attempt to access the hardware signing key without signing anything yet
#[derive(Debug, Parser)]
pub struct HsmAccess {
    #[clap(flatten)]
    pub pin: HsmPin,
}

#[derive(Debug, Parser)]
pub struct HsmPin {
    /// The pw1 pin to unlock signing operations
    #[arg(short = 'p', long = "pin")]
    pub value: Option<String>,
    /// Path to read pw1 pin from
    #[arg(long = "pin-file")]
    pub file: Option<PathBuf>,
}

/// Compile an attack based on a plot
#[derive(Debug, Clone, Parser)]
pub struct Build {
    #[clap(flatten)]
    pub plot: Plot,
    /// Run the build in this directory
    #[arg(short = 'C', long)]
    pub context: Option<PathBuf>,
    /// Output the compiled plot here
    #[arg(short, long)]
    pub output: Option<PathBuf>,
    /// Only load the source plot but do not process anything
    #[arg(short, long)]
    pub no_build: bool,
}

/// Check if the plot can still execute correctly against the configured image
#[derive(Debug, Clone, Parser)]
pub struct Check {
    #[clap(flatten)]
    pub plot: Plot,
    /// Address to bind to
    #[arg(short = 'B', long, env = "SH4D0WUP_BIND")]
    pub bind: Option<SocketAddr>,
    /// Pull the image even if it already exists locally
    #[arg(long)]
    pub pull: bool,
    /// Only load the plot but don't execute it
    #[arg(short, long, action(ArgAction::Count))]
    pub no_exec: u8,
    /// Keep the container running after the test is done instead of killing it immediately
    #[arg(short = 'K', long)]
    pub keep: bool,
}

/// Emulate a http request to test routing and selectors
#[derive(Debug, Clone, Parser)]
pub struct Req {
    #[clap(flatten)]
    pub plot: Plot,
    /// The path of the emulated http request
    #[arg(value_name = "PATH")]
    pub req_path: String,
    /// The http method to use
    #[arg(short = 'X', long, default_value = "GET")]
    pub method: String,
    #[arg(short = 'A', long)]
    pub authority: Option<warp::host::Authority>,
    #[arg(short = 'H', long = "header")]
    pub headers: Vec<String>,
    #[arg(long)]
    pub addr: Option<IpAddr>,
    /// Show the full generated response for the emulated request
    #[arg(short = 'r', long = "response")]
    pub show_response: bool,
    /// Show the generated status code for the emulated request
    #[arg(short = 's', long = "status")]
    pub show_status: bool,
    /// Show the generated headers for the emulated request
    #[arg(long = "headers")]
    pub show_headers: bool,
    /// Show the generated response for the emulated request
    #[arg(short = 'c', long = "content")]
    pub show_content: bool,
    /// When showing the response, use `hexdump -C`
    #[arg(short = 'C', long)]
    pub hexdump: bool,
}

/// Generate shell completions
#[derive(Debug, Parser)]
pub struct Completions {
    pub shell: Shell,
}

pub fn gen_completions(args: &Completions) -> Result<()> {
    clap_complete::generate(args.shell, &mut Args::command(), "sh4d0wup", &mut stdout());
    Ok(())
}
