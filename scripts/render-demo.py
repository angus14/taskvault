#!/usr/bin/env python3
"""Render the passing sandbox trace into a short, explicitly recorded demo video."""
import json
import pathlib
import subprocess
import tempfile
from PIL import Image, ImageDraw, ImageFont

ROOT = pathlib.Path(__file__).resolve().parents[1]
trace = json.loads((ROOT / "evidence/last-run.json").read_text())
out = ROOT.parent / "TaskVault_demo.mp4"
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
mint, white, muted, coral = "#B8F379", "#F2F8EF", "#A9C0B4", "#FFAE98"


def font(size, bold=False):
    return ImageFont.truetype(BOLD if bold else FONT, size)


def wrap(draw, text, width, f):
    lines, line = [], ""
    for word in text.split():
        trial = (line + " " + word).strip()
        if draw.textbbox((0, 0), trial, font=f)[2] > width and line:
            lines.append(line)
            line = word
        else:
            line = trial
    return lines + [line]


def frame(title, subtitle, event=None, note=""):
    im = Image.new("RGB", (1280, 720), "#081513")
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((35, 32, 1245, 688), radius=26, fill="#142622", outline="#345047", width=2)
    d.text((78, 67), "TASKVAULT  /  NEAR SANDBOX", font=font(22, True), fill=mint)
    d.text((78, 114), title, font=font(43, True), fill=white)
    for i, line in enumerate(wrap(d, subtitle, 1095, font(26))):
        d.text((78, 177 + i * 35), line, font=font(26), fill=muted)
    if event:
        y = 296
        for n, value in enumerate((event["spent"], event["reserved"], event["available"])):
            x = 78 + n * 378
            d.rounded_rectangle((x, y, x + 347, y + 160), radius=17, fill="#203B30", outline="#345047", width=2)
            d.text((x + 24, y + 18), ["SPENT", "RESERVED", "AVAILABLE"][n], font=font(19, True), fill=muted)
            d.text((x + 24, y + 55), str(value), font=font(56, True), fill=mint)
            d.text((x + 126, y + 115), "milliNEAR", font=font(17), fill=muted)
        outcome = event["outcome"]
        d.text((78, 496), "RESULT", font=font(19, True), fill=muted)
        d.text((78, 525), outcome, font=font(27, True), fill=coral if outcome.startswith(("rejected", "failed")) else mint)
        d.text((78, 578), "SIGNED TRANSACTION  " + event["tx"], font=font(19), fill=white)
        d.text((78, 619), f"Signer: {event['actor']}    |    Vendor received: {event['vendor_delta']} milliNEAR", font=font(18), fill=muted)
    else:
        d.text((78, 500), note, font=font(25, True), fill=mint)
    d.text((78, 658), "Recorded local run · reproduce with bash run.sh · no mainnet funds", font=font(16), fill=muted)
    return im


with tempfile.TemporaryDirectory() as tmp:
    path = pathlib.Path(tmp)
    slides = [(frame("One job. Several agents.", "Buyer authorizes 100 from a 1,000 milliNEAR treasury.", note="Every step below comes from a passing NEAR sandbox transaction."), 3)]
    for e in trace["events"]:
        slides.append((frame(e["action"], "Project " + e["project"] + " · on-chain state after this signed transaction", e),
                       3 if e["step"] in ("first_paid", "handoff", "transfer_failed", "fallback_paid", "unused_returned", "race") else 2))
    slides.append((frame("95 paid. 5 returned.", "The agent can finish the approved job without a second allowance or duplicate payment.", note="Source and complete machine-readable trace: evidence/last-run.json"), 4))
    manifest = path / "frames.txt"
    with manifest.open("w") as file:
        for i, (im, duration) in enumerate(slides):
            filename = path / f"slide-{i:02}.png"
            im.save(filename)
            file.write(f"file '{filename}'\nduration {duration}\n")
        file.write(f"file '{path / f'slide-{len(slides)-1:02}.png'}'\n")
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0",
                    "-i", str(manifest), "-r", "24", "-c:v", "libx264", "-pix_fmt", "yuv420p",
                    "-movflags", "+faststart", str(out)], check=True)
print(out)
