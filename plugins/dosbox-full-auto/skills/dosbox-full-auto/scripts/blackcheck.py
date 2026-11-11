#!/usr/bin/env python3
# This file is part of the dosbox-automation-is-not-your-nanny fork
# License: GPL-2.0-or-later. This file is original work of the fork. Contact: EternalNooblet <67196335+EternalNooblet@users.noreply.github.com>.

"""All-black frame detector for DOSBox captures. No vision — pixel math only.
Usage: blackcheck.py <png-path>
Prints BLACK (every pixel near pure #000000) or NOT_BLACK with max/mean.
"""
import sys
import numpy as np
from PIL import Image

img = np.asarray(Image.open(sys.argv[1]).convert("RGB"))
mx = int(img.max())
mean = float(img.mean())
# "all black" = strictly near (0,0,0); any gray/white UI pixel pushes max well above threshold.
print("BLACK" if mx <= 8 else f"NOT_BLACK max={mx} mean={mean:.1f}")
