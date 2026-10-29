#!/usr/bin/env python3
"""Stage a native fnOS app without changing the OpenWrt package files."""
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[1]


def prepare(binary, destination):
    destination = Path(destination)
    shutil.copytree(ROOT/'fnos',destination,dirs_exist_ok=True,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
    (destination/'wizard').mkdir(exist_ok=True)
    app=destination/'app'
    (app/'bin').mkdir(exist_ok=True)
    shutil.copy2(binary,app/'bin/wlmb-fnos')
    (app/'bin/wlmb-fnos').chmod(0o755)
    for size,name in ((64,'ICON.PNG'),(256,'ICON_256.PNG')):
        logo=app/'ui/images'/f'wlmb-logo-v2_{size}.png'
        shutil.copy2(logo,destination/name)
        # Retain the old path while the desktop metadata refreshes after upgrade.
        shutil.copy2(logo,app/'ui/images'/f'icon_{size}.png')
    for name in ('install_init','install_callback','upgrade_callback','config_init','config_callback','uninstall_callback'):
        (destination/'cmd'/name).write_text('#!/bin/bash\nexit 0\n',encoding='utf-8',newline='\n')
    for name in ('uninstall_init','upgrade_init'):
        (destination/'cmd'/name).write_text('#!/bin/bash\nexec "$(dirname "$0")/main" stop\n',encoding='utf-8',newline='\n')
    for file in (destination/'cmd').iterdir():file.chmod(0o755)
    assets=ROOT/'frontend/www/luci-static/resources/wlmb-server'
    for name in ('panel.css','webui-data-1.1.3.js','webui-icons-1.1.3.js','webui-shell-1.1.3.js'):
        shutil.copyfile(assets/name,app/'www'/name)
    notices=app/'licenses';notices.mkdir(exist_ok=True)
    shutil.copy2(ROOT/'LICENSE',notices/'LICENSE')
    shutil.copy2(ROOT/'packaging/files/usr/share/wlmb-server/THIRD-PARTY-NOTICES',notices/'THIRD-PARTY-NOTICES')


if __name__ == '__main__':prepare(*sys.argv[1:])
