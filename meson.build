project(
  'deskphone',
  'rust',
  version: '0.1.0',
  meson_version: '>= 0.59.0',
  license: 'GPL-3.0-or-later',
)

i18n = import('i18n')
gnome = import('gnome')

application_id = 'com.martingawron.Deskphone'
gettext_package = 'deskphone'

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
localedir = prefix / get_option('localedir')

cargo = find_program('cargo', required: true)

subdir('data')
subdir('po')
subdir('src')

gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
