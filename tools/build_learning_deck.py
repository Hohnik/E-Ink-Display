"""Add word-by-word glosses and kanji cues to the Japanese deck."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT.parent / "eink_python" / "decks" / "japanese_100.json"
OUTPUT = ROOT / "decks" / "japanese_100.json"

# Surface form: (contextual reading, short direct English gloss).
LEXICON = {
    "7時": ("しちじ", "seven o'clock"),
    "9時": ("くじ", "nine o'clock"),
    "5時": ("ごじ", "five o'clock"),
    "お願いします": ("おねがいします", "please / request"),
    "すみません": ("すみません", "excuse me / sorry"),
    "じゃない": ("じゃない", "be not"),
    "ありません": ("ありません", "not exist / not have"),
    "分かります": ("わかります", "understand (polite)"),
    "違います": ("ちがいます", "differ / be wrong"),
    "待って": ("まって", "wait (connecting)"),
    "待ちます": ("まちます", "wait (polite)"),
    "食べました": ("たべました", "ate (polite)"),
    "日本人": ("にほんじん", "Japanese person"),
    "新しい": ("あたらしい", "new"),
    "あります": ("あります", "exist / have (polite)"),
    "します": ("します", "do (polite)"),
    "来ます": ("きます", "come (polite)"),
    "会いました": ("あいました", "met (polite)"),
    "会います": ("あいます", "meet (polite)"),
    "食べます": ("たべます", "eat (polite)"),
    "帰ります": ("かえります", "return home (polite)"),
    "住みます": ("すみます", "live (polite)"),
    "買います": ("かいます", "buy (polite)"),
    "話します": ("はなします", "speak (polite)"),
    "読みます": ("よみます", "read (polite)"),
    "行きます": ("いきます", "go (polite)"),
    "います": ("います", "exist / be (polite)"),
    "見ます": ("みます", "see / watch (polite)"),
    "聞きます": ("ききます", "listen / ask (polite)"),
    "言います": ("いいます", "say (polite)"),
    "思います": ("おもいます", "think (polite)"),
    "飲みます": ("のみます", "drink (polite)"),
    "起きます": ("おきます", "wake up (polite)"),
    "働きます": ("はたらきます", "work (polite)"),
    "できます": ("できます", "can / be able"),
    "降って": ("ふって", "fall (connecting)"),
    "ください": ("ください", "please give"),
    "ありがとう": ("ありがとう", "thank you"),
    "ございます": ("ございます", "be (honorific)"),
    "大丈夫": ("だいじょうぶ", "okay / all right"),
    "ちょっと": ("ちょっと", "a little / moment"),
    "たぶん": ("たぶん", "probably"),
    "みんな": ("みんな", "everyone"),
    "あそこ": ("あそこ", "over there"),
    "どこ": ("どこ", "where"),
    "いつ": ("いつ", "when"),
    "どれ": ("どれ", "which one"),
    "いくら": ("いくら", "how much"),
    "とても": ("とても", "very"),
    "おいしい": ("おいしい", "delicious"),
    "日本語": ("にほんご", "Japanese language"),
    "ご飯": ("ごはん", "rice / meal"),
    "お茶": ("おちゃ", "tea"),
    "お金": ("おかね", "money"),
    "テレビ": ("てれび", "television"),
    "パン": ("ぱん", "bread"),
    "私": ("わたし", "I / me"),
    "は": ("わ", "topic"),
    "学生": ("がくせい", "student"),
    "です": ("です", "be; polite"),
    "雨": ("あめ", "rain"),
    "が": ("が", "subject"),
    "水": ("みず", "water"),
    "を": ("お", "object"),
    "に": ("に", "time / destination"),
    "家": ("いえ", "home / house"),
    "で": ("で", "at / by means of"),
    "友達": ("ともだち", "friend"),
    "と": ("と", "with / and"),
    "も": ("も", "also"),
    "の": ("の", "of / possessive"),
    "名前": ("なまえ", "name"),
    "学校": ("がっこう", "school"),
    "へ": ("え", "toward"),
    "から": ("から", "from"),
    "まで": ("まで", "until"),
    "か": ("か", "question"),
    "いい": ("いい", "good"),
    "ね": ("ね", "seeking agreement"),
    "よ": ("よ", "emphasis"),
    "今日": ("きょう", "today"),
    "休み": ("やすみ", "rest / day off"),
    "毎日": ("まいにち", "every day"),
    "もう": ("もう", "already / more"),
    "一度": ("いちど", "one time"),
    "はい": ("はい", "yes"),
    "いいえ": ("いいえ", "no"),
    "ここ": ("ここ", "here"),
    "まだ": ("まだ", "still / not yet"),
    "そう": ("そう", "so / that way"),
    "本当": ("ほんとう", "truth / really"),
    "これ": ("これ", "this"),
    "それ": ("それ", "that"),
    "本": ("ほん", "book"),
    "あれ": ("あれ", "that over there"),
    "駅": ("えき", "station"),
    "この": ("この", "this (before noun)"),
    "好き": ("すき", "liked / pleasing"),
    "その": ("その", "that (before noun)"),
    "店": ("みせ", "shop"),
    "あの": ("あの", "that over there"),
    "人": ("ひと", "person"),
    "誰": ("だれ", "who"),
    "そこ": ("そこ", "there"),
    "何": ("なに", "what"),
    "どう": ("どう", "how"),
    "どの": ("どの", "which (before noun)"),
    "なぜ": ("なぜ", "why"),
    "仕事": ("しごと", "work / job"),
    "明日": ("あした", "tomorrow"),
    "昨日": ("きのう", "yesterday"),
    "今": ("いま", "now"),
    "後": ("あと", "after / later"),
    "前": ("まえ", "before / in front"),
    "時間": ("じかん", "time"),
    "電話": ("でんわ", "telephone"),
    "家族": ("かぞく", "family"),
    "電車": ("でんしゃ", "train"),
    "日本": ("にほん", "Japan"),
    "天気": ("てんき", "weather"),
    "猫": ("ねこ", "cat"),
    "音楽": ("おんがく", "music"),
    "意味": ("いみ", "meaning"),
    "犬": ("いぬ", "dog"),
    "欲しい": ("ほしい", "wanted / desirable"),
    "大きい": ("おおきい", "big"),
    "小さい": ("ちいさい", "small"),
    "忙しい": ("いそがしい", "busy"),
    "早く": ("はやく", "early / quickly"),
    "値段": ("ねだん", "price"),
    "高い": ("たかい", "high / expensive"),
}

# More precise meanings where a grammatical word changes with its context.
OVERRIDES = {
    ("雨が降っています。", "います"): "be ongoing (polite)",
    ("7時に起きます。", "に"): "at (time)",
    ("家に帰ります。", "に"): "to (destination)",
    ("学校に行きます。", "に"): "to (destination)",
    ("その店に行きます。", "に"): "to (destination)",
    ("友達に会います。", "に"): "with (meeting target)",
    ("駅に行きます。", "に"): "to (destination)",
    ("日本に行きます。", "に"): "to (destination)",
    ("ここにいます。", "に"): "at (location)",
    ("そこにあります。", "に"): "at (location)",
    ("あそこに駅があります。", "に"): "at (location)",
    ("前に会いました。", "に"): "at (past time)",
    ("後で話します。", "で"): "at (later time)",
    ("家で食べます。", "で"): "at (place)",
    ("店で買います。", "で"): "at (place)",
    ("電車で行きます。", "で"): "by means of",
    ("みんなで行きます。", "で"): "as a group",
    ("もう一度お願いします。", "もう"): "one more",
    ("もう食べました。", "もう"): "already",
}

KANJI = {
    "私": "I / private", "学": "study", "生": "life / birth", "雨": "rain",
    "降": "fall", "水": "water", "飲": "drink", "時": "time / hour",
    "起": "rise / wake", "家": "house / home", "食": "eat", "友": "friend",
    "達": "attain / plural", "話": "speak / talk", "行": "go", "名": "name",
    "前": "front / before", "校": "school", "働": "work", "大": "big",
    "丈": "length / strong", "夫": "man / husband", "今": "now", "日": "day / sun",
    "休": "rest", "毎": "every", "一": "one", "度": "time / degree",
    "願": "wish / request", "駅": "station", "分": "part / understand",
    "違": "differ", "待": "wait", "本": "origin / book", "当": "true / hit",
    "人": "person", "新": "new", "好": "like", "店": "shop", "誰": "who",
    "何": "what", "来": "come", "仕": "serve / work", "事": "thing / matter",
    "明": "bright", "昨": "previous", "後": "after / behind", "会": "meet",
    "間": "interval", "飯": "meal / rice", "茶": "tea", "金": "gold / money",
    "帰": "return", "族": "family / tribe", "住": "reside", "電": "electricity",
    "車": "vehicle", "買": "buy", "語": "language / word", "読": "read",
    "天": "sky / heaven", "気": "spirit / feeling", "猫": "cat", "見": "see",
    "音": "sound", "楽": "music / ease", "聞": "hear / listen", "言": "say",
    "思": "think", "意": "idea / meaning", "味": "taste / meaning", "犬": "dog",
    "欲": "desire", "小": "small", "忙": "busy", "早": "early", "値": "value",
    "段": "step / grade", "高": "high / costly",
}

PUNCTUATION = "。、！？"


def gloss(sentence: str) -> list[dict[str, str]]:
    words = []
    at = 0
    forms = sorted(LEXICON, key=len, reverse=True)
    while at < len(sentence):
        if sentence[at] in PUNCTUATION:
            if not words:
                raise ValueError(f"Punctuation without word in {sentence!r}")
            words[-1]["japanese"] += sentence[at]
            at += 1
            continue
        form = next(
            (
                candidate
                for candidate in forms
                if sentence.startswith(candidate, at) and (candidate != "はい" or at == 0)
            ),
            None,
        )
        if form is None:
            raise ValueError(f"No gloss for {sentence[at:]!r} in {sentence!r}")
        reading, english = LEXICON[form]
        english = OVERRIDES.get((sentence, form), english)
        words.append({"japanese": form, "reading": reading, "english": english})
        at += len(form)
    return words


def kanji_meanings(card: dict) -> list[dict[str, str]]:
    seen = set()
    result = []
    for character in card["japanese"] + card["example"]["ja"]:
        if character in KANJI and character not in seen:
            seen.add(character)
            result.append({"kanji": character, "english": KANJI[character]})
    return result


def main() -> None:
    deck = json.loads(SOURCE.read_text())
    for card in deck["cards"]:
        card["example"]["words"] = gloss(card["example"]["ja"])
        card["kanji"] = kanji_meanings(card)
    OUTPUT.parent.mkdir(exist_ok=True)
    OUTPUT.write_text(json.dumps(deck, ensure_ascii=False, separators=(",", ":")) + "\n")
    print(f"Wrote {len(deck['cards'])} cards to {OUTPUT} ({OUTPUT.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
