/**
 * Browser speech synthesis defaults often sound noticeably robotic. This wrapper
 * improves them as far as the Web Speech API allows by:
 *
 * - waiting for asynchronously loaded voices and preferring premium, cloud-based,
 *   natural, neural, enhanced, or online English voices;
 * - lowering pitch to 0.9 and rate to 0.95 for a calmer delivery;
 * - removing asterisks and expanding dollar amounts into spoken words;
 * - replacing commas with ellipses to create more natural pauses; and
 * - speaking one sentence at a time to improve phrasing and cadence.
 *
 * Voice availability still depends on the browser and operating system.
 */
export class BrowserSpeaker {
  #preferredVoice = null;
  #requestedVoice;
  #voicesReady;

  constructor(voice = null) {
    this.#requestedVoice = voice;
    let markVoicesReady;
    this.#voicesReady = new Promise((resolve) => {
      markVoicesReady = resolve;
    });

    const loadVoices = () => {
      const voices = window.speechSynthesis.getVoices();
      if (voices.length === 0) return;

      this.#preferredVoice = this.#selectVoice(voices);
      markVoicesReady();
    };

    window.speechSynthesis.addEventListener("voiceschanged", loadVoices);
    loadVoices();
  }

  async speak(text) {
    if (typeof text !== "string" || !text.trim()) return;

    await Promise.race([
      this.#voicesReady,
      new Promise((resolve) => setTimeout(resolve, 1_000)),
    ]);

    const normalizedText = normalizeSpeechText(text);
    for (const sentence of splitSentences(normalizedText)) {
      await this.#speakSentence(sentence.replaceAll(",", "..."));
    }
  }

  #selectVoice(voices) {
    if (this.#requestedVoice) {
      const requestedVoice = voices.find(
        (voice) => voice.name === this.#requestedVoice,
      );
      if (requestedVoice) return requestedVoice;
    }

    const englishVoices = voices.filter((voice) =>
      voice.lang.toLowerCase().startsWith("en"),
    );
    const premiumName = /(natural|neural|premium|enhanced|online)/i;

    return englishVoices.find(
      (voice) => !voice.localService && premiumName.test(voice.name),
    )
      ?? englishVoices.find((voice) => premiumName.test(voice.name))
      ?? englishVoices.find((voice) => !voice.localService)
      ?? englishVoices.find((voice) => voice.default)
      ?? englishVoices[0]
      ?? null;
  }

  #speakSentence(sentence) {
    return new Promise((resolve, reject) => {
      const utterance = new SpeechSynthesisUtterance(sentence);
      if (this.#preferredVoice) utterance.voice = this.#preferredVoice;
      utterance.pitch = 0.9;
      utterance.rate = 0.95;
      utterance.addEventListener("end", resolve, { once: true });
      utterance.addEventListener(
        "error",
        (event) => reject(new Error("Browser speech failed: " + event.error)),
        { once: true },
      );
      window.speechSynthesis.speak(utterance);
    });
  }

}
function normalizeSpeechText(text) {
  return text.replaceAll("*", "").replace(
    /\$(\d+(?:,\d{3})*)(?:\.(\d{1,2}))?(?!\d|\.\d)/g,
    (_match, dollarText, centText) => {
      const dollars = Number.parseInt(dollarText.replaceAll(",", ""), 10);
      const cents = centText
        ? Number.parseInt(centText.padEnd(2, "0"), 10)
        : 0;
      const parts = [];

      if (dollars !== 0 || cents === 0) {
        parts.push(dollars + " " + (dollars === 1 ? "dollar" : "dollars"));
      }
      if (cents !== 0) {
        parts.push(cents + " " + (cents === 1 ? "cent" : "cents"));
      }

      return parts.join(" and ");
    },
  );
}

function splitSentences(text) {
  return text.match(/[^.!?]+(?:[.!?]+|$)/g)?.map((sentence) => sentence.trim())
    .filter(Boolean) ?? [];
}
