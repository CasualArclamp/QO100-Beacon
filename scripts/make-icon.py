# made by CasualArclamp
# python scripts/make-icon.py assets   (needs numpy and Pillow)
# The app's icon: the multimedia beacon's 8APSK constellation, one point at
# the centre and seven on a ring, as the app's heat map draws it.
import sys, math, numpy as np
from PIL import Image
S = 1024
yy, xx = np.mgrid[0:S, 0:S].astype(np.float32)
heat = np.zeros((S, S), np.float32)
R = 0.30 * S
pts = [(S / 2, S / 2)] + [(S / 2 + R * math.cos(2 * math.pi * k / 7 - math.pi / 2), S / 2 + R * math.sin(2 * math.pi * k / 7 - math.pi / 2)) for k in range(7)]
for (px, py) in pts:
    d2 = (xx - px) ** 2 + (yy - py) ** 2
    heat += np.exp(-d2 / (2 * (0.052 * S) ** 2)) + 0.35 * np.exp(-d2 / (2 * (0.10 * S) ** 2))
heat = np.clip(heat / heat.max(), 0, 1) ** 0.8
stops = [(0.0, (0, 0, 4)), (0.2, (40, 11, 84)), (0.4, (101, 21, 110)), (0.6, (159, 42, 99)), (0.8, (237, 105, 37)), (1.0, (252, 255, 164))]
rgb = np.zeros((S, S, 3), np.float32)
for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
    m = (heat >= t0) & (heat <= t1)
    f = ((heat - t0) / (t1 - t0))[m][:, None]
    rgb[m] = np.array(c0) + f * (np.array(c1) - np.array(c0))
img = np.dstack([rgb, np.full((S, S), 255, np.float32)]).astype(np.uint8)
# A square tile with a thin edge, as the app's panels.
e = S // 48
img[:e, :, :3] = img[-e:, :, :3] = (58, 61, 66); img[:, :e, :3] = img[:, -e:, :3] = (58, 61, 66)
im = Image.fromarray(img, 'RGBA')
im.resize((256, 256), Image.LANCZOS).save(sys.argv[1] + '/icon.png')
im.save(sys.argv[1] + '/icon.ico', sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
