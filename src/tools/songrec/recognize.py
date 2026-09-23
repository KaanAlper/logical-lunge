# ii scripts/musicRecognition/recognize-music.sh + services/SongRec.qml karşılığı (Windows).
# ii Linux'ta `songrec` (Shazam istemcisi) ile varsayılan çıkışın monitöründen dinler; burada aynı Shazam
# servisine shazamio ile soruyoruz, sesi WASAPI loopback (hoparlörden çalan ses) ile alıyoruz.
#   recognize.py [-i 2] [-t 30] [-s monitor|input]
# Bulursa tek satır JSON: {"title","subtitle","url","cover"}; bulamazsa {"error":"notfound"}.
import argparse, asyncio, io, json, sys, time, wave

import pyaudiowpatch as pyaudio
from shazamio import Shazam

ap = argparse.ArgumentParser()
ap.add_argument("-i", type=float, default=2)    # istekler arası süre
ap.add_argument("-t", type=float, default=30)   # toplam deneme süresi
ap.add_argument("-s", default="monitor")        # monitor: çalan ses, input: mikrofon
args = ap.parse_args()


def out(obj):
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def open_stream(pa):
    if args.s == "input":
        dev = pa.get_default_input_device_info()
    else:
        wasapi = pa.get_host_api_info_by_type(pyaudio.paWASAPI)
        spk = pa.get_device_info_by_index(wasapi["defaultOutputDevice"])
        dev = spk
        if not spk.get("isLoopbackDevice"):
            for lb in pa.get_loopback_device_info_generator():
                if spk["name"] in lb["name"]:
                    dev = lb
                    break
    ch = max(1, min(2, int(dev["maxInputChannels"]) or 2))
    rate = int(dev["defaultSampleRate"])
    # WASAPI loopback sessizken hiç veri vermez; engelleyen read() takılır. Geri çağırmayla topla.
    chunks = []

    def cb(data, n, info, status):
        chunks.append(data)
        return (None, pyaudio.paContinue)

    st = pa.open(format=pyaudio.paInt16, channels=ch, rate=rate, input=True,
                 input_device_index=dev["index"], frames_per_buffer=1024, stream_callback=cb)
    st.start_stream()
    return st, ch, rate, chunks


def to_wav(frames, ch, rate):
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(ch); w.setsampwidth(2); w.setframerate(rate)
        w.writeframes(b"".join(frames))
    return buf.getvalue()


async def main():
    pa = pyaudio.PyAudio()
    try:
        st, ch, rate, chunks = open_stream(pa)
    except Exception as e:
        out({"error": "audio", "detail": str(e)}); return 1
    shazam = Shazam()
    frames, start, last = [], time.time(), 0.0
    window = 8.0  # Shazam için son ~8 sn ses
    try:
        while time.time() - start < args.t:
            await asyncio.sleep(0.1)
            while chunks:
                frames.append(chunks.pop(0))
            keep = int(window * rate / 1024)
            if len(frames) > keep:
                frames = frames[-keep:]
            elapsed = time.time() - start
            if elapsed >= 4 and len(frames) > int(3 * rate / 1024) and time.time() - last >= args.i:
                last = time.time()
                try:
                    res = await shazam.recognize(to_wav(frames, ch, rate))
                except Exception:
                    continue
                tr = res.get("track") if isinstance(res, dict) else None
                if tr:
                    out({"title": tr.get("title", ""), "subtitle": tr.get("subtitle", ""),
                         "url": tr.get("url", ""), "cover": (tr.get("images") or {}).get("coverart", "")})
                    return 0
        out({"error": "notfound"})
        return 0
    finally:
        st.close(); pa.terminate()


sys.exit(asyncio.run(main()))
