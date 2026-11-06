#!/usr/bin/env python3
"""Blank the build time and date in a dumped ESP-IDF app descriptor.

The descriptor (`esp_app_desc_t`, section `.rodata_desc`) records when the
firmware was built, so two builds of the same code never hash the same. This
zeroes just those fields in a copy used for hashing. It checks the layout
first and fails instead of blanking the wrong bytes if it ever changes.

Usage: blank-app-desc-time.py <dumped .rodata_desc file>
"""

import re
import struct
import sys

DESC_SIZE = 256
MAGIC_WORD = 0xABCD5432  # ESP_APP_DESC_MAGIC_WORD
TIME = slice(80, 96)  # "HH:MM:SS", NUL padded
DATE = slice(96, 112)  # "YYYY-MM-DD", NUL padded
TIME_SHAPE = re.compile(rb"\d{2}:\d{2}:\d{2}\x00+")
DATE_SHAPE = re.compile(rb"\d{4}-\d{2}-\d{2}\x00+")


def fail(message):
    sys.exit(f"app descriptor layout check failed: {message}")


path = sys.argv[1]
desc = bytearray(open(path, "rb").read())

if len(desc) != DESC_SIZE:
    fail(f"expected {DESC_SIZE} bytes, found {len(desc)}")
(magic,) = struct.unpack_from("<I", desc, 0)
if magic != MAGIC_WORD:
    fail(f"expected magic word {MAGIC_WORD:#x}, found {magic:#x}")
if not TIME_SHAPE.fullmatch(desc[TIME]):
    fail(f"bytes 80-95 are not a build time: {bytes(desc[TIME])!r}")
if not DATE_SHAPE.fullmatch(desc[DATE]):
    fail(f"bytes 96-111 are not a build date: {bytes(desc[DATE])!r}")

desc[TIME] = bytes(TIME.stop - TIME.start)
desc[DATE] = bytes(DATE.stop - DATE.start)
open(path, "wb").write(desc)
