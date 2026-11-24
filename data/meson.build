install_data(
  'com.martingawron.Deskphone.desktop',
  install_dir: datadir / 'applications',
)

desktop_utils = find_program('desktop-file-validate', required: false)
if desktop_utils.found()
  test(
    'validate-desktop',
    desktop_utils,
    args: [files('com.martingawron.Deskphone.desktop')],
  )
endif

install_data(
  'com.martingawron.Deskphone.metainfo.xml',
  install_dir: datadir / 'metainfo',
)

appstreamcli = find_program('appstreamcli', required: false)
if appstreamcli.found()
  test(
    'validate-metainfo',
    appstreamcli,
    args: ['validate', '--no-net', '--explain', files('com.martingawron.Deskphone.metainfo.xml')],
  )
endif

install_data(
  'icons/hicolor/scalable/apps/com.martingawron.Deskphone.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'scalable' / 'apps',
)

install_data(
  'icons/hicolor/symbolic/apps/com.martingawron.Deskphone-symbolic.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'symbolic' / 'apps',
)
