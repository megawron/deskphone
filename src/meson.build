cargo_options = [
  '--manifest-path', meson.project_source_root() / 'Cargo.toml',
  '--target-dir', meson.project_build_root() / 'target',
  '--bin', 'deskphone',
]

if get_option('buildtype') == 'release' or get_option('buildtype') == 'plain'
  cargo_options += ['--release']
  rust_target = 'release'
else
  rust_target = 'debug'
endif

cargo_env = environment()
cargo_env.set('DESKPHONE_LOCALEDIR', localedir)
cargo_env.set('CARGO_TARGET_DIR', meson.project_build_root() / 'target')

custom_target(
  'deskphone-bin',
  build_by_default: true,
  build_always_stale: true,
  output: 'deskphone',
  console: true,
  install: true,
  install_dir: bindir,
  env: cargo_env,
  command: [
    'sh', '-c',
    'cargo build "$@" && cp "$CARGO_TARGET_DIR/' + rust_target + '/deskphone" "$0"',
    '@OUTPUT@',
    cargo_options,
  ],
)
