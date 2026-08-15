use std::path::PathBuf;

use anyhow::{Context, Result};
use audiopus::{Channels, SampleRate, coder::Decoder};
use yamabiko_whisper::{OnlineAsrModel, OnlineAsrProcessor, VadModel, Word};

const DEFAULT_WHISPER_SAMPLE_RATE: usize = 16_000;
const DEFAULT_MAX_OPUS_FRAME_SAMPLES: usize = 1_920;

#[derive(Clone, Debug, Default)]
pub struct STTDecoderBuilder {
    language: Option<String>,
    whisper_model_path: Option<PathBuf>,
    vad_model_path: Option<PathBuf>,
    whisper_sample_rate: Option<usize>,
    max_opus_frame_samples: Option<usize>,
}

impl STTDecoderBuilder {
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    pub fn whisper_model_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.whisper_model_path = Some(path.into());
        self
    }

    pub fn vad_model_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.vad_model_path = Some(path.into());
        self
    }

    pub fn whisper_sample_rate(mut self, sample_rate: usize) -> Self {
        self.whisper_sample_rate = Some(sample_rate);
        self
    }

    pub fn max_opus_frame_samples(mut self, samples: usize) -> Self {
        self.max_opus_frame_samples = Some(samples);
        self
    }

    pub fn build(self) -> Result<STTDecoder> {
        let language = self.language.context("STT language is required")?;
        let whisper_model_path = self
            .whisper_model_path
            .context("Whisper model path is required")?;
        let vad_model_path = self.vad_model_path.context("VAD model path is required")?;
        let whisper_sample_rate = self
            .whisper_sample_rate
            .unwrap_or(DEFAULT_WHISPER_SAMPLE_RATE);
        let max_opus_frame_samples = self
            .max_opus_frame_samples
            .unwrap_or(DEFAULT_MAX_OPUS_FRAME_SAMPLES);

        let whisper_model = OnlineAsrModel::load(&whisper_model_path)
            .context("failed to load Whisper model")?;
        let vad_model =
            VadModel::load(&vad_model_path).context("failed to load Silero VAD model")?;
        let whisper_processor = whisper_model
            .create_processor_with_vad(&language, &vad_model)
            .context("failed to create Whisper processor")?;
        let separator = whisper_processor.sep().to_owned();
        let opus_decoder = Decoder::new(opus_sample_rate(whisper_sample_rate)?, Channels::Mono)
            .context("failed to create Opus decoder")?;

        Ok(STTDecoder {
            opus_decoder,
            whisper_processor,
            separator,
            whisper_sample_rate,
            pcm_buffer: vec![0.0; max_opus_frame_samples],
            pending_pcm: Vec::with_capacity(whisper_sample_rate + max_opus_frame_samples),
        })
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct StreamOutput {
    pub tentative_text: Option<String>,
    pub committed_text: Option<String>,
}

impl StreamOutput {
    pub fn has_text(&self) -> bool {
        self.tentative_text.is_some() || self.committed_text.is_some()
    }
}

pub struct STTDecoder {
    opus_decoder: Decoder,
    whisper_processor: OnlineAsrProcessor,
    separator: String,
    whisper_sample_rate: usize,
    pcm_buffer: Vec<f32>,
    pending_pcm: Vec<f32>,
}

impl STTDecoder {
    pub fn builder() -> STTDecoderBuilder {
        STTDecoderBuilder::default()
    }

    pub fn stream(&mut self, rtp_buffer: &[u8]) -> Result<StreamOutput> {
        let decoded_samples = self
            .opus_decoder
            .decode_float(Some(rtp_buffer), &mut self.pcm_buffer, false)
            .context("failed to decode Opus payload")?;
        self.pending_pcm
            .extend_from_slice(&self.pcm_buffer[..decoded_samples]);

        if self.pending_pcm.len() < self.whisper_sample_rate {
            return Ok(StreamOutput::default());
        }

        self.whisper_processor
            .insert_audio_chunk(&self.pending_pcm)
            .context("failed to buffer PCM for Whisper")?;
        self.pending_pcm.clear();
        let output = self
            .whisper_processor
            .process()
            .context("Whisper inference failed")?;

        Ok(StreamOutput {
            tentative_text: join_words(&output.tentative, &self.separator),
            committed_text: join_words(&output.committed, &self.separator),
        })
    }
}

fn join_words(words: &[Word], separator: &str) -> Option<String> {
    (!words.is_empty()).then(|| {
        words
            .iter()
            .map(|word| word.text.as_str())
            .collect::<Vec<_>>()
            .join(separator)
    })
}

fn opus_sample_rate(sample_rate: usize) -> Result<SampleRate> {
    match sample_rate {
        8_000 => Ok(SampleRate::Hz8000),
        12_000 => Ok(SampleRate::Hz12000),
        16_000 => Ok(SampleRate::Hz16000),
        24_000 => Ok(SampleRate::Hz24000),
        48_000 => Ok(SampleRate::Hz48000),
        _ => anyhow::bail!("unsupported Opus sample rate: {sample_rate}"),
    }
}
