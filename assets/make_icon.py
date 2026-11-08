"""Draws the app icon: a green rounded square with a white "D".

Run from the project root:  python assets/make_icon.py
Writes assets/icon.png (window icon) and assets/icon.ico (exe icon).
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

SIZE = 256
GREEN = (46, 125, 50, 255)
WHITE = (255, 255, 255, 255)
FONT = Path("C:/Windows/Fonts/segoeuib.ttf")
OUT = Path(__file__).resolve().parent


def draw() -> Image.Image:
    image = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    canvas = ImageDraw.Draw(image)
    canvas.rounded_rectangle((8, 8, SIZE - 8, SIZE - 8), radius=52, fill=GREEN)
    font = ImageFont.truetype(str(FONT), 190)
    left, top, right, bottom = canvas.textbbox((0, 0), "D", font=font)
    x = (SIZE - (right - left)) / 2 - left
    y = (SIZE - (bottom - top)) / 2 - top
    canvas.text((x, y), "D", font=font, fill=WHITE)
    return image


def main() -> None:
    image = draw()
    image.save(OUT / "icon.png")
    image.save(OUT / "icon.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])


if __name__ == "__main__":
    main()
