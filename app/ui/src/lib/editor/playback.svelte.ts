// Playback controller for the editor.
//
// It drives a real <video> element when the clip has a playable URL and falls back to a
// clock-driven simulation for the mock backend (`mock://` URLs), so every control in the
// editor works in a plain browser. Reverse shuttle (J) is driven by seeking, since media
// elements do not play backwards.

import type { TrackAudio } from '$lib/api/types';
import { TrackMixer, whenPlayable } from './mixer';

const MAX_SPEED = 8;

export class Playback {
	/** Playhead position in seconds. */
	time = $state(0);
	playing = $state(false);
	/** Signed shuttle speed: 1 is normal, -2 is reverse at double speed. */
	speed = $state(1);
	duration = $state(0);
	ready = $state(false);
	error = $state<string | null>(null);
	volume = $state(1);
	muted = $state(false);
	loop = $state(false);
	/**
	 * How the preview plays audio: `pending` while track files load, `tracks` when each track
	 * plays separately (per-track volume is audible), `video` when only the recorded mix is
	 * available, `none` for the mock's simulated playback.
	 */
	audioMode = $state<'pending' | 'tracks' | 'video' | 'none'>('pending');
	/**
	 * After a jump while playing, everything holds still until the picture and every track
	 * have data at the new spot, then starts together: a short wait instead of a stutter.
	 */
	buffering = $state(false);

	readonly simulated: boolean;
	fps: number;

	#video: HTMLVideoElement | null = null;
	#raf = 0;
	#timer: ReturnType<typeof setTimeout> | null = null;
	#last = 0;
	#stopAt = 0;
	#rangeIn = 0;
	#rangeOut = 0;
	#pendingSeek: number | null = null;
	#mixer: TrackMixer | null = null;
	#volumes: Record<number, number> = {};
	#lastSync = 0;
	/** Ranges that playback jumps over, sorted, in seconds. */
	#skips: { start: number; end: number }[] = [];
	/** Ranges played at another speed, as the export will be. */
	#speeds: { start: number; end: number; speed: number }[] = [];
	/** The rate the video plays at right now: shuttle speed times the range's speed. */
	#rate = 1;
	/** Called when playing stops at the end of the trimmed range (not when looping). */
	onReachedEnd: (() => void) | null = null;
	/** Start playing as soon as the video can, for going on into the next clip. */
	#playOnReady = false;
	/** Counts holds, so only the latest of several quick jumps starts playback again. */
	#hold = 0;

	constructor(url: string, duration: number, fps: number) {
		this.simulated = url.startsWith('mock:');
		this.duration = duration;
		this.fps = Math.max(1, fps);
		this.#rangeOut = duration;
		if (this.simulated) {
			this.ready = true;
			this.audioMode = 'none';
		}
	}

	get frame(): number {
		return Math.round(this.time * this.fps);
	}

	setRange(rangeIn: number, rangeOut: number) {
		this.#rangeIn = rangeIn;
		this.#rangeOut = rangeOut;
	}

	/** Ranges that playing jumps over, like the parts of a clip that are cut out. */
	setSkips(ranges: { start: number; end: number }[]) {
		this.#skips = [...ranges].sort((a, b) => a.start - b.start);
	}

	/** Ranges played faster or slower. The pitch follows, like the export's audio. */
	setSpeeds(ranges: { start: number; end: number; speed: number }[]) {
		this.#speeds = [...ranges].sort((a, b) => a.start - b.start);
		if (this.playing) this.#applyRate();
	}

	/** The speed of the range at `t`, 1 outside them. */
	speedAt(t: number): number {
		return this.#speeds.find((r) => t >= r.start && t < r.end)?.speed ?? 1;
	}

	// ---------- <video> wiring ----------

	attach(video: HTMLVideoElement | null) {
		if (this.#video) {
			this.#video.removeEventListener('seeking', this.#onSeeking);
			this.#video.removeEventListener('waiting', this.#onWaiting);
		}
		this.#video = video;
		if (!video) return;
		video.addEventListener('seeking', this.#onSeeking);
		video.addEventListener('waiting', this.#onWaiting);
		video.volume = this.volume;
		// Slowed and sped-up ranges change the pitch, as they will in the export.
		video.preservesPitch = false;
		// With separate track audio the video is silent and the mixer plays instead.
		video.muted = this.#mixer ? true : this.muted;
		if (this.#playOnReady && video.readyState > 0) {
			this.#playOnReady = false;
			this.play();
		}
	}

	/**
	 * A seek or a stall while playing: the video decodes from the keyframe before the new
	 * spot and each track seeks on its own, finishing at different times. Played through,
	 * the audio ran ahead and was pulled back again and again: a stutter. So everything stops,
	 * waits until the picture and all tracks are ready there, and starts together.
	 */
	#holdUntilReady(aheadSecs = 1.5) {
		const video = this.#video;
		if (!video || !this.playing || this.speed <= 0) return;
		const hold = ++this.#hold;
		this.buffering = true;
		video.pause();
		this.#mixer?.pause();
		const target = this.time;
		void Promise.all([whenPlayable(video, 6000, aheadSecs), this.#mixer?.ready(target) ?? Promise.resolve()]).then(() => {
			if (hold !== this.#hold) return;
			this.buffering = false;
			if (!this.playing || this.speed <= 0 || this.#video !== video) return;
			this.#last = performance.now();
			video.playbackRate = this.#rate;
			void video.play().catch(() => {
				this.playing = false;
			});
			this.#mixer?.play(video.currentTime, this.#rate);
			this.#lastSync = performance.now();
		});
	}
	/** A seek not started here (the browser's own controls, for example). */
	#onSeeking = () => {
		if (this.playing && !this.buffering) this.#holdUntilReady();
	};
	/** The video ran out of data while playing: the tracks wait with it. */
	#onWaiting = () => {
		if (this.playing && !this.buffering && this.#video && !this.#video.seeking) {
			this.time = this.#video.currentTime;
			// It ran dry once already: wait for a bit more, so it does not again at once.
			this.#holdUntilReady(1.8);
		}
	};

	/** Plays now if the video is loaded, otherwise as soon as it is. */
	playWhenReady() {
		if (this.simulated || (this.#video && this.#video.readyState > 0)) this.play();
		else this.#playOnReady = true;
	}

	onMetadata() {
		const video = this.#video;
		if (!video) return;
		if (Number.isFinite(video.duration) && video.duration > 0) this.duration = video.duration;
		this.ready = true;
		this.error = null;
		if (this.#pendingSeek !== null) {
			video.currentTime = this.#pendingSeek;
			this.#pendingSeek = null;
		} else if (this.time > 0) {
			video.currentTime = this.time;
		}
		if (this.#playOnReady) {
			this.#playOnReady = false;
			this.play();
		}
	}

	onError() {
		this.error = 'This clip could not be played. The file may be missing or use a codec the webview cannot decode.';
		this.ready = false;
		this.pause();
	}

	// ---------- Audio ----------

	/**
	 * Uses one audio file per track for the preview. An empty list, or any track failing to
	 * load, falls back to the video's own mixed audio.
	 */
	async useTracks(tracks: TrackAudio[]) {
		if (this.simulated) return;
		if (tracks.length === 0) {
			this.audioMode = 'video';
			return;
		}
		try {
			const mixer = await TrackMixer.create(tracks);
			if (this.audioMode !== 'pending') {
				mixer.dispose();
				return;
			}
			this.#mixer = mixer;
			for (const [index, volume] of Object.entries(this.#volumes)) mixer.setTrackVolume(Number(index), volume);
			mixer.setMaster(this.volume, this.muted);
			if (this.#video) this.#video.muted = true;
			if (this.playing && this.speed > 0) mixer.play(this.time, this.speed);
			this.audioMode = 'tracks';
		} catch {
			this.audioMode = 'video';
		}
	}

	/** Track volumes, 0 to 2. Only audible with separate track audio. */
	setTrackVolumes(volumes: Record<number, number>) {
		this.#volumes = { ...volumes };
		if (!this.#mixer) return;
		for (const [index, volume] of Object.entries(volumes)) this.#mixer.setTrackVolume(Number(index), volume);
	}

	setVolume(volume: number) {
		this.volume = Math.min(1, Math.max(0, volume));
		if (this.#mixer) this.#mixer.setMaster(this.volume, this.muted);
		else if (this.#video) this.#video.volume = this.volume;
	}

	setMuted(muted: boolean) {
		this.muted = muted;
		if (this.#mixer) this.#mixer.setMaster(this.volume, muted);
		else if (this.#video) this.#video.muted = muted;
	}

	// ---------- Transport ----------

	seek(seconds: number) {
		const t = Math.min(Math.max(0, seconds), this.duration);
		this.time = t;
		const video = this.#video;
		if (video && video.readyState > 0) video.currentTime = t;
		else if (video) this.#pendingSeek = t;
		if (video && this.playing && this.speed > 0 && video.readyState > 0) this.#holdUntilReady();
		else this.#mixer?.seek(t);
	}

	/**
	 * Moves by whole frames and pauses, like frame stepping in an editor. The target sits a
	 * hair past the frame's start so the decoder never shows the frame before it.
	 */
	step(frames: number) {
		this.pause();
		const target = (Math.round(this.time * this.fps) + frames) / this.fps;
		this.seek(target + (target > 0 ? 1e-4 : 0));
	}

	/** Jumps by `seconds` and keeps playing if it was. */
	skip(seconds: number) {
		this.seek(this.time + seconds);
		// With a video everything starts again once the seek is done (see #holdUntilReady).
		if (!this.#video && this.playing && this.speed > 0) this.#mixer?.play(this.time, this.#rate);
	}

	stepSeconds(seconds: number) {
		this.pause();
		this.seek(this.time + seconds);
	}

	play() {
		if (this.error) return;
		const eps = 0.5 / this.fps;
		this.#stopAt = this.time < this.#rangeOut - eps ? this.#rangeOut : this.duration;
		if (this.time >= this.#stopAt - eps) {
			this.seek(this.#rangeIn);
			this.#stopAt = this.#rangeOut;
		}
		if (this.speed <= 0) this.speed = 1;
		this.#run();
	}

	pause() {
		this.playing = false;
		this.speed = 1;
		this.#hold++;
		this.buffering = false;
		this.#video?.pause();
		this.#mixer?.pause();
		this.#cancelTick();
	}

	toggle() {
		if (this.playing) this.pause();
		else this.play();
	}

	/** L: play forward, doubling the speed on each further press. */
	shuttleForward() {
		if (!this.playing || this.speed <= 0) {
			this.speed = 1;
			this.playing = false;
			this.play();
			return;
		}
		this.speed = Math.min(MAX_SPEED, this.speed * 2);
		this.#applyRate();
	}

	/** J: play backwards, doubling the speed on each further press. */
	shuttleReverse() {
		if (this.playing && this.speed < 0) this.speed = Math.max(-MAX_SPEED, this.speed * 2);
		else this.speed = -1;
		this.#stopAt = 0;
		this.#run();
	}

	dispose() {
		this.#hold++;
		this.buffering = false;
		this.#cancelTick();
		this.#video?.pause();
		this.#mixer?.dispose();
		this.#mixer = null;
		this.audioMode = this.simulated ? 'none' : 'pending';
		this.#video = null;
		this.playing = false;
	}

	// ---------- Internals ----------

	#applyRate() {
		if (this.speed <= 0) return;
		this.#rate = Math.min(16, Math.max(0.0625, this.speed * this.speedAt(this.time)));
		if (this.#video) this.#video.playbackRate = this.#rate;
		this.#mixer?.setRate(this.#rate);
	}

	#run() {
		this.playing = true;
		this.#last = performance.now();
		const video = this.#video;
		if (video) {
			if (this.speed > 0) {
				this.#rate = Math.min(16, Math.max(0.0625, this.speed * this.speedAt(this.time)));
				video.playbackRate = this.#rate;
				// Not ready at this spot yet (just jumped there): start together once it is.
				if (video.seeking || video.readyState < HTMLMediaElement.HAVE_FUTURE_DATA) {
					this.#holdUntilReady();
					this.#schedule();
					return;
				}
				void video.play().catch(() => {
					// Autoplay policy or a decode failure; the error event reports real failures.
					this.playing = false;
				});
			} else {
				video.pause();
			}
		}
		// Reverse shuttle is silent; forward playback keeps the track audio with the picture.
		if (this.speed > 0) this.#mixer?.play(this.time, this.#rate);
		else this.#mixer?.pause();
		this.#schedule();
	}

	/** Frame-aligned when the window is painting, with a timer fallback when frames are not delivered. */
	#schedule() {
		this.#cancelTick();
		this.#raf = requestAnimationFrame(this.#tick);
		this.#timer = setTimeout(() => this.#tick(performance.now()), 34);
	}

	#cancelTick() {
		cancelAnimationFrame(this.#raf);
		this.#raf = 0;
		if (this.#timer) clearTimeout(this.#timer);
		this.#timer = null;
	}

	#tick = (now: number) => {
		if (!this.playing) return;
		this.#cancelTick();
		const dt = Math.min(0.25, (now - this.#last) / 1000);
		this.#last = now;
		const video = this.#video;

		if (this.speed < 0) {
			const next = this.time + dt * this.speed;
			if (next <= 0) {
				this.seek(0);
				this.pause();
				return;
			}
			this.seek(next);
		} else if (this.buffering) {
			// Holding at the new spot until everything can play from there.
		} else {
			this.time = video && !video.paused ? video.currentTime : Math.min(this.duration, this.time + dt * this.speed);
			if (video?.ended || this.time >= this.duration) {
				this.seek(this.duration);
				this.pause();
				return;
			}
			if (this.#stopAt > 0 && this.time >= this.#stopAt) {
				if (this.loop && this.#stopAt === this.#rangeOut) {
					this.seek(this.#rangeIn);
				} else {
					const atEnd = this.#stopAt === this.#rangeOut;
					this.seek(this.#stopAt);
					this.pause();
					if (atEnd) this.onReachedEnd?.();
					return;
				}
			}
			// Jump over a cut-out part; the export will not have it either.
			const skip = this.#skips.find((r) => this.time >= r.start && this.time < r.end - 0.001);
			if (skip) {
				this.seek(skip.end);
				this.#mixer?.sync(this.time);
			}
			// Entering or leaving a range at another speed.
			if (Math.abs(this.speed * this.speedAt(this.time) - this.#rate) > 1e-6) this.#applyRate();
			// Keep the separate track audio within 50 ms of the picture (not while the video is
			// still seeking: its time stands still then).
			if (this.#mixer && !video?.seeking && now - this.#lastSync > 250) {
				this.#lastSync = now;
				this.#mixer.sync(this.time);
			}
		}
		this.#schedule();
	};
}
