// Per-track audio preview for the editor.
//
// Each track from `getTrackAudio` plays in its own <audio> element routed through a GainNode, so
// a track can be previewed from 0 to 200 % while the <video> itself stays muted. The playback
// controller keeps the elements in step with the video and corrects drift above 50 ms.

import type { TrackAudio } from '$lib/api/types';

const DRIFT_SECS = 0.05;

/** Seconds of media buffered from the current position on. */
export function bufferedAhead(el: HTMLMediaElement): number {
	const t = el.currentTime;
	for (let i = 0; i < el.buffered.length; i++) {
		if (el.buffered.start(i) <= t + 0.05 && el.buffered.end(i) >= t) return el.buffered.end(i) - t;
	}
	return 0;
}

/**
 * Resolves once `el` is not seeking and has `aheadSecs` of data from where it is (less near
 * the end), or after `timeoutMs`. Starting on just the next frame ran dry again at once with
 * big recordings: a stutter of short stops.
 */
export function whenPlayable(el: HTMLMediaElement, timeoutMs: number, aheadSecs = 0): Promise<void> {
	const done = () => {
		if (el.seeking || el.readyState < HTMLMediaElement.HAVE_FUTURE_DATA) return false;
		const left = Number.isFinite(el.duration) ? el.duration - el.currentTime - 0.05 : aheadSecs;
		return bufferedAhead(el) >= Math.min(aheadSecs, Math.max(0, left));
	};
	if (done()) return Promise.resolve();
	return new Promise((resolve) => {
		const events = ['seeked', 'canplay', 'canplaythrough', 'loadeddata', 'progress', 'error'] as const;
		const check = () => {
			if (done() || el.error) finish();
		};
		const finish = () => {
			clearTimeout(timer);
			clearInterval(poll);
			for (const e of events) el.removeEventListener(e, check);
			resolve();
		};
		const timer = setTimeout(finish, timeoutMs);
		// Buffering does not always fire an event; look again now and then.
		const poll = setInterval(check, 100);
		for (const e of events) el.addEventListener(e, check);
	});
}

interface Channel {
	index: number;
	el: HTMLAudioElement;
	gain: GainNode;
}

export class TrackMixer {
	#ctx: AudioContext;
	#master: GainNode;
	#channels: Channel[] = [];
	#disposed = false;

	private constructor(ctx: AudioContext) {
		this.#ctx = ctx;
		this.#master = ctx.createGain();
		this.#master.connect(ctx.destination);
	}

	/**
	 * Loads every track. Rejects if any track cannot be loaded (missing file, blocked by CORS),
	 * in which case the caller falls back to the video's own mixed audio.
	 */
	static async create(tracks: TrackAudio[]): Promise<TrackMixer> {
		const mixer = new TrackMixer(new AudioContext());
		try {
			await Promise.all(tracks.map((t) => mixer.#add(t)));
			return mixer;
		} catch (error) {
			mixer.dispose();
			throw error;
		}
	}

	#add(track: TrackAudio): Promise<void> {
		return new Promise((resolve, reject) => {
			const el = new Audio();
			// Sped-up and slowed parts change pitch, as in the export.
			el.preservesPitch = false;
			// Web Audio only hears cross-origin media that is served with CORS.
			el.crossOrigin = 'anonymous';
			el.preload = 'auto';
			const cleanup = () => {
				el.removeEventListener('canplay', ok);
				el.removeEventListener('error', fail);
			};
			const ok = () => {
				cleanup();
				resolve();
			};
			const fail = () => {
				cleanup();
				reject(new Error(`Track ${track.index + 1} could not be loaded.`));
			};
			el.addEventListener('canplay', ok);
			el.addEventListener('error', fail);
			el.src = track.url;
			const source = this.#ctx.createMediaElementSource(el);
			const gain = this.#ctx.createGain();
			source.connect(gain);
			gain.connect(this.#master);
			this.#channels.push({ index: track.index, el, gain });
		});
	}

	setTrackVolume(index: number, volume: number) {
		const channel = this.#channels.find((c) => c.index === index);
		if (channel) channel.gain.gain.value = Math.max(0, volume);
	}

	setMaster(volume: number, muted: boolean) {
		this.#master.gain.value = muted ? 0 : volume;
	}

	play(time: number, rate: number) {
		if (this.#disposed) return;
		void this.#ctx.resume();
		for (const c of this.#channels) {
			c.el.playbackRate = rate;
			if (Math.abs(c.el.currentTime - time) > DRIFT_SECS) c.el.currentTime = time;
			void c.el.play().catch(() => {});
		}
	}

	pause() {
		for (const c of this.#channels) c.el.pause();
	}

	seek(time: number) {
		for (const c of this.#channels) c.el.currentTime = time;
	}

	/**
	 * Moves every track to `time` and resolves once each one has finished seeking and has
	 * enough to play (or after `timeoutMs`, so a slow track never holds playback forever).
	 */
	ready(time: number, timeoutMs = 4000): Promise<void> {
		return Promise.all(
			this.#channels.map((c) => {
				if (Math.abs(c.el.currentTime - time) > DRIFT_SECS / 2) c.el.currentTime = time;
				return whenPlayable(c.el, timeoutMs, 0.5);
			})
		).then(() => {});
	}

	setRate(rate: number) {
		for (const c of this.#channels) c.el.playbackRate = rate;
	}

	/** Pulls any track more than 50 ms away from the video back into line. */
	sync(time: number) {
		for (const c of this.#channels) {
			if (!c.el.paused && Math.abs(c.el.currentTime - time) > DRIFT_SECS) c.el.currentTime = time;
		}
	}

	dispose() {
		this.#disposed = true;
		for (const c of this.#channels) {
			c.el.pause();
			c.el.removeAttribute('src');
			c.el.load();
		}
		this.#channels = [];
		void this.#ctx.close().catch(() => {});
	}
}
