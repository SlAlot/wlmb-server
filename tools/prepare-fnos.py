#!/usr/bin/env python3
"""Stage a native fnOS app without changing the OpenWrt package files."""
from pathlib import Path
import shutil
import struct
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]


def icon(size):
    def chunk(kind, value):
        return struct.pack('!I', len(value)) + kind + value + struct.pack('!I', zlib.crc32(kind + value))
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            a, b = x * 64 / size, y * 64 / size
            white = (12 <= a < 52 and (12 <= b < 17 or 47 <= b < 52) or
                     12 <= b < 52 and (12 <= a < 17 or 47 <= a < 52) or
                     22 <= b < 43 and (abs(a-b+3)<2 or abs(a+b-61)<2))
            rows.extend((255,255,255) if white else (52,133,255))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!IIBBBBB',size,size,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(rows))+chunk(b'IEND',b'')


def prepare(binary, destination):
    destination = Path(destination)
    shutil.copytree(ROOT/'fnos',destination,dirs_exist_ok=True,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
    (destination/'wizard').mkdir(exist_ok=True)
    app=destination/'app'
    (app/'bin').mkdir(exist_ok=True)
    shutil.copy2(binary,app/'bin/wlmb-fnos')
    (app/'bin/wlmb-fnos').chmod(0o755)
    for size,name in ((64,'ICON.PNG'),(256,'ICON_256.PNG')):
        data=icon(size);(destination/name).write_bytes(data)
        images=app/'ui/images';images.mkdir(parents=True,exist_ok=True);(images/f'icon_{size}.png').write_bytes(data)
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
