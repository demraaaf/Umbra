// One-sentence explanations for terms a normal person may not know. Written like a manual:
// what the thing is and what changing it does. Used by Settings and the recording panel.

import type { RateMode, VideoCodec } from '$lib/api/types';

export const EXPLAIN = {
	bitrate:
		'Bitrate is how much data each second of video may use; more means sharper motion and larger files.',
	rateControl: 'Rate control decides how the encoder spends the bitrate from one second to the next.',
	qp: 'The quantiser (QP) sets how much detail the encoder may throw away; lower keeps more detail and makes larger files.',
	preset: 'The encoder preset trades speed for efficiency; slower presets give better quality at the same bitrate but load the GPU more.',
	codec: 'The codec is the compression format of the video.',
	buffer: 'The replay buffer keeps the last seconds of gameplay in memory so a clip can be saved after something happens.',
	droppedFrames: 'Dropped frames are frames the encoder could not finish in time; they are missing from the recording.',
	encoder: 'The encoder is the part of the graphics card that compresses video; its name shows which one is in use.',
	encoderLoad:
		'Encoder load is how busy the graphics card’s video encoder would be; above 100% it cannot keep up and frames get dropped.',
	cpu: 'CPU is the share of the whole processor Umbra uses.',
	memory: 'Memory is the RAM Umbra uses, including the replay buffer.',
	gpu: 'GPU is the share of the graphics card’s 3D and compute work Umbra uses, which games compete for.',
	encoderUsage: 'Encoder is how busy the graphics card’s separate video encoder is; games do not use it.',
	audioBitrate: 'Audio bitrate is how much data each audio track uses; 160 kbit/s is transparent for most voices and game sound.',
	pushToTalk: 'With push to talk, the microphone is only recorded while you hold the key.',
	clipSize: 'Clip size is how large one saved clip will be on disk.',
	screenshotFormat: 'PNG keeps every pixel exactly; JPG files are several times smaller and lose a little detail.',
	scaling:
		'When the picture has another shape than the video, such as a game at a stretched 1440×1080 on a 16:9 screen: stretch it to fill, the way your screen shows it, or keep its shape with black bars.',
	tune: 'Tuning tells the encoder what to favour. High quality is right for recording; the low-latency modes are meant for streaming and save little here.',
	multipass:
		'Multipass lets the encoder look at each frame once before encoding it, so bits go where they matter. It costs encoder speed: about a quarter at quarter resolution, about half at full.',
	lookahead:
		'Look-ahead lets the encoder see upcoming frames to plan the bitrate around scene changes. It uses a little memory and delays each frame by that many frames.',
	spatialAq:
		'Spatial adaptive quantisation moves bits from busy detail, where losses hide, to flat areas like skies and walls, where blocks show.',
	temporalAq: 'Temporal adaptive quantisation spends more on parts of the picture that stay still, so they stay sharp over time.',
	aqStrength: 'How far spatial AQ shifts the bits: 1 barely, 15 strongly. 8 suits most games.',
	skipStill:
		'When nothing on screen moves, Umbra encodes the same picture again for every frame. Leaving those out lets the encoder rest on a still desktop; clips look the same, but their frame rate varies, which some editors such as Premiere and DaVinci Resolve handle badly.',
	keyframes:
		'A keyframe is a full picture the video can start from. Clips start on one, so a shorter interval makes clip starts more exact; a longer one saves a little bitrate.'
} as const;

export const RATE_MODE_EXPLAIN: Record<RateMode, string> = {
	cbr: 'CBR (constant bitrate) uses the same amount of data every second, so file size and buffer memory are predictable.',
	vbr: 'VBR (variable bitrate) averages the target and may rise to the peak during busy scenes, so fast action stays sharper.',
	cq: 'CQ (constant quality) keeps the picture at one quality level and lets the size follow the scene, never going past the peak bitrate.',
	cqp: 'CQP (constant QP) compresses every frame by the same fixed amount with no limit, so busy scenes make much larger files.'
};

export const CODEC_EXPLAIN: Record<VideoCodec, string> = {
	h264: 'H.264 plays on almost every device and site, but needs the most bitrate for a given quality.',
	hevc: 'HEVC (H.265) gives similar quality at roughly half the bitrate of H.264; most modern players support it.',
	av1: 'AV1 is the most efficient of the three, but older players and some editors cannot open it yet.'
};

/** Video bitrate presets, in kbit/s, with a two-word label each. */
export const VIDEO_BITRATE_PRESETS = [
	{ kbps: 10000, label: 'Small files' },
	{ kbps: 20000, label: 'Everyday use' },
	{ kbps: 35000, label: 'Good quality' },
	{ kbps: 50000, label: 'High quality' },
	{ kbps: 80000, label: 'Very high' },
	{ kbps: 130000, label: 'Archive grade' }
] as const;

export const AUDIO_BITRATE_PRESETS = [96, 128, 160, 192, 256, 320] as const;
