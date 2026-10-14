"""Mise en forme des réponses : json, text, verbose_json, srt, vtt (comme l'API OpenAI)."""

from __future__ import annotations

from enum import Enum
from typing import Any

from murmure_server.engine import TranscriptionResult

# Noms de langue renvoyés par OpenAI dans `verbose_json` (liste de Whisper).
LANGUAGE_NAMES: dict[str, str] = {
    "en": "english", "zh": "chinese", "de": "german", "es": "spanish", "ru": "russian",
    "ko": "korean", "fr": "french", "ja": "japanese", "pt": "portuguese", "tr": "turkish",
    "pl": "polish", "ca": "catalan", "nl": "dutch", "ar": "arabic", "sv": "swedish",
    "it": "italian", "id": "indonesian", "hi": "hindi", "fi": "finnish", "vi": "vietnamese",
    "he": "hebrew", "uk": "ukrainian", "el": "greek", "ms": "malay", "cs": "czech",
    "ro": "romanian", "da": "danish", "hu": "hungarian", "ta": "tamil", "no": "norwegian",
    "th": "thai", "ur": "urdu", "hr": "croatian", "bg": "bulgarian", "lt": "lithuanian",
    "la": "latin", "mi": "maori", "ml": "malayalam", "cy": "welsh", "sk": "slovak",
    "te": "telugu", "fa": "persian", "lv": "latvian", "bn": "bengali", "sr": "serbian",
    "az": "azerbaijani", "sl": "slovenian", "kn": "kannada", "et": "estonian", "mk": "macedonian",
    "br": "breton", "eu": "basque", "is": "icelandic", "hy": "armenian", "ne": "nepali",
    "mn": "mongolian", "bs": "bosnian", "kk": "kazakh", "sq": "albanian", "sw": "swahili",
    "gl": "galician", "mr": "marathi", "pa": "punjabi", "si": "sinhala", "km": "khmer",
    "sn": "shona", "yo": "yoruba", "so": "somali", "af": "afrikaans", "oc": "occitan",
    "ka": "georgian", "be": "belarusian", "tg": "tajik", "sd": "sindhi", "gu": "gujarati",
    "am": "amharic", "yi": "yiddish", "lo": "lao", "uz": "uzbek", "fo": "faroese",
    "ht": "haitian creole", "ps": "pashto", "tk": "turkmen", "nn": "nynorsk", "mt": "maltese",
    "sa": "sanskrit", "lb": "luxembourgish", "my": "myanmar", "bo": "tibetan", "tl": "tagalog",
    "mg": "malagasy", "as": "assamese", "tt": "tatar", "haw": "hawaiian", "ln": "lingala",
    "ha": "hausa", "ba": "bashkir", "jw": "javanese", "su": "sundanese", "yue": "cantonese",
}


class ResponseFormat(str, Enum):
    json = "json"
    text = "text"
    verbose_json = "verbose_json"
    srt = "srt"
    vtt = "vtt"


def _timestamp(seconds: float, decimal_marker: str) -> str:
    ms_total = max(0, round(seconds * 1000))
    hours, rem = divmod(ms_total, 3_600_000)
    minutes, rem = divmod(rem, 60_000)
    secs, ms = divmod(rem, 1000)
    return f"{hours:02d}:{minutes:02d}:{secs:02d}{decimal_marker}{ms:03d}"


def to_srt(result: TranscriptionResult) -> str:
    blocks = [
        f"{i}\n{_timestamp(s.start, ',')} --> {_timestamp(s.end, ',')}\n{s.text.strip()}\n"
        for i, s in enumerate(result.segments, start=1)
    ]
    return "\n".join(blocks)


def to_vtt(result: TranscriptionResult) -> str:
    blocks = ["WEBVTT\n"]
    blocks += [
        f"{_timestamp(s.start, '.')} --> {_timestamp(s.end, '.')}\n{s.text.strip()}\n"
        for s in result.segments
    ]
    return "\n".join(blocks)


def to_verbose_json(result: TranscriptionResult) -> dict[str, Any]:
    return {
        "task": "transcribe",
        "language": LANGUAGE_NAMES.get(result.language, result.language),
        "duration": result.duration,
        "text": result.text,
        "segments": [
            {
                "id": s.id,
                "seek": s.seek,
                "start": s.start,
                "end": s.end,
                "text": s.text,
                "tokens": s.tokens,
                "temperature": s.temperature,
                "avg_logprob": s.avg_logprob,
                "compression_ratio": s.compression_ratio,
                "no_speech_prob": s.no_speech_prob,
            }
            for s in result.segments
        ],
    }
