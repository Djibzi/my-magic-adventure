"""
Génère l'atlas de textures bloc pour My Magic Adventure.
11 tuiles 16x16 px → 176x16.
Usage : python gen_textures.py
Sortie : assets/blocks.png
"""

from PIL import Image
import random
import math
import os

TILE = 16
TILES = 11
W = TILE * TILES   # 176
H = TILE           # 16

# Seed fixe pour un atlas deterministe
random.seed(42)

def noise(x, y, seed=0):
    h = (x * 73856093) ^ (y * 19349663) ^ (seed * 6271)
    h = (h ^ (h >> 5) ^ (h >> 13)) & 0xFFFF
    return h / 0xFFFF

def clamp(v, lo=0, hi=255):
    return max(lo, min(hi, int(v)))

def vary(base, amount, x, y, seed=0):
    n = noise(x, y, seed)
    return clamp(base + (n - 0.5) * 2 * amount)

# ---------------------------------------------------------------------------
# Tuile 0 — Herbe (dessus)
# ---------------------------------------------------------------------------
def tile_grass_top(x, y):
    r = vary(106, 20, x, y, 1)
    g = vary(148, 22, x, y, 2)
    b = vary(50,  14, x, y, 3)
    if noise(x, y, 7) > 0.82:
        r = clamp(r - 20)
        g = clamp(g - 25)
        b = clamp(b - 10)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Tuile 1 — Herbe (côté)
# ---------------------------------------------------------------------------
def tile_grass_side(x, y):
    if y <= 2:
        r = vary(106, 16, x, y, 1)
        g = vary(148, 18, x, y, 2)
        b = vary(50,  12, x, y, 3)
        if noise(x, y, 9) > 0.70:
            r = clamp(r - 18)
            g = clamp(g - 22)
    elif y == 3:
        blend = noise(x, y, 5)
        if blend > 0.5:
            r = vary(106, 14, x, y, 1)
            g = vary(140, 16, x, y, 2)
            b = vary(52,  10, x, y, 3)
        else:
            r = vary(134, 14, x, y, 4)
            g = vary(96,  12, x, y, 5)
            b = vary(67,  10, x, y, 6)
    else:
        r = vary(134, 16, x, y, 4)
        g = vary(96,  12, x, y, 5)
        b = vary(67,  10, x, y, 6)
        if noise(x, y, 11) > 0.88:
            r = clamp(r + 22)
            g = clamp(g + 18)
            b = clamp(b + 14)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Tuile 2 — Terre
# ---------------------------------------------------------------------------
def tile_dirt(x, y):
    r = vary(134, 18, x, y, 4)
    g = vary(96,  14, x, y, 5)
    b = vary(67,  12, x, y, 6)
    if noise(x, y, 12) > 0.86:
        r = clamp(r + 20)
        g = clamp(g + 16)
        b = clamp(b + 12)
    if noise(x, y, 13) > 0.90:
        r = clamp(r - 18)
        g = clamp(g - 14)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Tuile 3 — Pierre
# ---------------------------------------------------------------------------
def tile_stone(x, y):
    base = vary(128, 22, x, y, 8)
    crack = noise(x * 2, y * 2, 14)
    if crack > 0.78:
        base = clamp(base - 28)
    if noise(x, y, 15) > 0.90:
        base = clamp(base + 20)
    return (base, base, clamp(base + vary(0, 4, x, y, 16)))

# ---------------------------------------------------------------------------
# Tuile 4 — Sable
# ---------------------------------------------------------------------------
def tile_sand(x, y):
    r = vary(220, 14, x, y, 17)
    g = vary(210, 14, x, y, 18)
    b = vary(158, 10, x, y, 19)
    if noise(x, y, 20) > 0.82:
        r = clamp(r - 16)
        g = clamp(g - 16)
        b = clamp(b - 10)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Tuile 5 — Neige
# ---------------------------------------------------------------------------
def tile_snow(x, y):
    w = vary(248, 6, x, y, 21)
    b = clamp(w + vary(4, 4, x, y, 22))
    if noise(x, y, 23) > 0.80:
        w = clamp(w - 14)
        b = clamp(b - 6)
    return (w, w, b)

# ---------------------------------------------------------------------------
# Tuile 6 — Bois (cote) - ecorce sombre avec rainures verticales
# ---------------------------------------------------------------------------
def tile_wood_side(x, y):
    # Base marron-gris
    r = vary(96,  14, x, y, 30)
    g = vary(64,  10, x, y, 31)
    b = vary(38,   8, x, y, 32)
    # Rainures verticales (colonnes sombres periodiques)
    col = (x * 7 + int(noise(0, y // 3, 33) * 8)) % 5
    if col == 0:
        r = clamp(r - 22)
        g = clamp(g - 16)
        b = clamp(b - 10)
    # Noeuds occasionnels
    if noise(x // 3, y // 4, 34) > 0.90:
        r = clamp(r - 30)
        g = clamp(g - 24)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Tuile 7 — Bois (dessus) - cernes de croissance
# ---------------------------------------------------------------------------
def tile_wood_top(x, y):
    cx, cy = 7.5, 7.5
    dx = x - cx
    dy = y - cy
    dist = math.sqrt(dx * dx + dy * dy)
    ring = int(dist) % 2
    if ring == 0:
        r = vary(138, 12, x, y, 35)
        g = vary(96,  10, x, y, 36)
        b = vary(52,   8, x, y, 37)
    else:
        r = vary(108, 12, x, y, 38)
        g = vary(72,   8, x, y, 39)
        b = vary(38,   6, x, y, 40)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Tuile 8 — Feuilles - vert sombre avec trous
# ---------------------------------------------------------------------------
def tile_leaves(x, y):
    r = vary(42, 14, x, y, 41)
    g = vary(108, 22, x, y, 42)
    b = vary(38, 10, x, y, 43)
    # Taches sombres
    if noise(x, y, 44) > 0.68:
        r = clamp(r - 18)
        g = clamp(g - 30)
        b = clamp(b - 16)
    # Points clairs
    if noise(x, y, 45) > 0.90:
        r = clamp(r + 14)
        g = clamp(g + 26)
        b = clamp(b + 12)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Tuile 9 — Planches - bois raffine avec lignes horizontales
# ---------------------------------------------------------------------------
def tile_planks(x, y):
    r = vary(170, 16, x, y, 46)
    g = vary(128, 12, x, y, 47)
    b = vary(72,  10, x, y, 48)
    # Lignes de separation tous les 4 pixels (y)
    if y % 4 == 0:
        r = clamp(r - 30)
        g = clamp(g - 22)
        b = clamp(b - 14)
    # Noeuds aleatoires
    if noise(x, y, 49) > 0.92:
        r = clamp(r - 25)
        g = clamp(g - 18)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Tuile 10 — Glace - bleu clair cristallin
# ---------------------------------------------------------------------------
def tile_ice(x, y):
    r = vary(170, 10, x, y, 50)
    g = vary(210, 10, x, y, 51)
    b = vary(245,  8, x, y, 52)
    # Craquelures cristallines
    if noise(x * 2, y, 53) > 0.85 or noise(x, y * 2, 54) > 0.85:
        r = clamp(r - 24)
        g = clamp(g - 14)
    # Reflets tres clairs
    if noise(x, y, 55) > 0.92:
        r = clamp(r + 20)
        g = clamp(g + 14)
        b = clamp(b + 6)
    return (r, g, b)

# ---------------------------------------------------------------------------
# Génération de l'atlas
# ---------------------------------------------------------------------------

TILE_FUNCS = [
    tile_grass_top,
    tile_grass_side,
    tile_dirt,
    tile_stone,
    tile_sand,
    tile_snow,
    tile_wood_side,
    tile_wood_top,
    tile_leaves,
    tile_planks,
    tile_ice,
]

img = Image.new("RGBA", (W, H))
pixels = img.load()

for t, fn in enumerate(TILE_FUNCS):
    for py in range(H):
        for px in range(TILE):
            r, g, b = fn(px, py)
            pixels[t * TILE + px, py] = (r, g, b, 255)

os.makedirs("assets", exist_ok=True)
img.save("assets/blocks.png")
print(f"Atlas genere : assets/blocks.png ({W}x{H} px, {TILES} tuiles de {TILE}x{TILE})")

big = img.resize((W * 8, H * 8), Image.NEAREST)
big.save("assets/blocks_preview.png")
print(f"Apercu x8    : assets/blocks_preview.png ({W*8}x{H*8} px)")
