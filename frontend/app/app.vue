<template>
  <ClientOnly>
    <div class="flex justify-center items-center gap-4 p-4 bg-slate-900 text-slate-200">
      <div class="flex flex-col items-center gap-2">
        <span>Live Sync</span>
        <span class="font-mono font-semibold">{{ hms(liveSync) }}</span>
      </div>
      <div class="flex flex-col items-center gap-2">
        <span>Live Delta</span>
        <span class="font-mono font-semibold">{{ liveDelta >= 0 ? '+' +
          liveDelta.toFixed(1) + 's' : '-' + Math.abs(liveDelta).toFixed(1) + 's' }}</span>
      </div>
    </div>

    <div class="max-w-5xl mx-auto p-4">
      <div ref="playerEl" class="relative bg-black border border-slate-800 rounded-2xl overflow-hidden shadow-2xl"
        @mousemove="onMouseMove" @mouseleave="onMouseLeave" @keydown="onKeydown" tabindex="0">
        <video ref="videoEl" class="w-full h-auto bg-black" playsinline preload="metadata"></video>

        <!-- Controls -->
        <div
          class="absolute inset-x-0 bottom-0 bg-gradient-to-b from-transparent via-slate-900/80 to-slate-900/95 p-3 space-y-2 transition-opacity duration-300"
          :class="{ 'opacity-0 pointer-events-none': !controlsVisible }">
          <div class="flex items-center gap-2">
            <button @click="togglePlay"
              class="px-3 py-2 rounded-full bg-slate-800 text-slate-100 font-semibold hover:brightness-110"
              :aria-label="isPlaying ? 'Pause' : 'Play'">
              <span v-if="!isPlaying">
                <LucidePlay class="size-4" />
              </span>
              <span v-else>
                <LucidePause class="size-4" />
              </span>
            </button>

            <!-- Mute toggle -->
            <button @click="toggleMute"
              class="px-3 py-2 rounded-full bg-slate-800 text-slate-100 font-semibold hover:brightness-110"
              aria-label="Ton stummschalten">
              <span v-if="muted || volume === 0">
                <LucideVolumeOff class="size-4" />
              </span>
              <span v-else>
                <LucideVolume2 class="size-4" />
              </span>
            </button>

            <!-- Volume slider -->
            <div class="flex items-center gap-2 select-none">
              <input type="range" min="0" max="1" step="0.01" v-model.number="volume" @input="onVolumeInput"
                class="w-32 h-2 bg-slate-800 rounded-full appearance-none cursor-pointer"
                :aria-label="`Lautstärke: ${Math.round(volume * 100)}%`" />
              <span class="text-slate-400 tabular-nums w-10 text-right">{{ Math.round(volume * 100) }}%</span>
            </div>

            <div class="flex-1"></div>

            <div v-if="isEvent" class="flex items-center gap-2">
              <span class="px-2.5 py-1.5 rounded-full border border-slate-700 bg-slate-800 text-slate-200 text-sm font-medium
           flex items-center gap-1.5 select-none" :class="{ 'opacity-70': !atLiveEdge }" :title="lagLabel">
                <span class="rounded-full inline-block"
                  :class="atLiveEdge ? 'bg-emerald-400' : (liveDelta >= 0 ? 'bg-amber-400' : 'bg-cyan-400')"
                  style="width:.5rem;height:.5rem" />
                <span>Live</span>
                <span v-if="!atLiveEdge" class="text-slate-400">
                  {{ liveDelta >= 0 ? '+' + liveDelta.toFixed(1) + 's' : '-' + Math.abs(liveDelta).toFixed(1) + 's' }}
                </span>
              </span>

              <button v-show="Math.abs(liveDelta) >= 10.0" @click="goLive" class="px-3 py-1.5 rounded-full border border-slate-700 bg-slate-800 text-slate-100 text-sm font-semibold
           hover:bg-slate-700/60 focus:outline-none focus:ring-2 focus:ring-emerald-400/40">
                Zum Live-Punkt
              </button>
            </div>
          </div>

          <!-- DVR bar -->
          <div class="w-full h-2 rounded-full bg-slate-800 relative cursor-pointer" @pointerdown="onBarPointer">
            <div class="absolute inset-y-0 left-0 bg-slate-700" :style="{ width: bufferedPct + '%' }"></div>
            <div class="absolute inset-y-0 left-0 bg-gradient-to-r from-emerald-300 to-cyan-300"
              :style="{ width: playedPct + '%' }"></div>
            <div
              class="absolute top-1/2 -translate-y-1/2 rounded-full bg-white shadow-[0_0_0_3px_rgba(255,255,255,0.2)]"
              :style="{ left: playedPct + '%', width: '14px', height: '14px', transform: 'translate(-50%,-50%)' }">
            </div>
          </div>

          <div class="flex items-center gap-2 flex-wrap">
            <span class="text-slate-400 tabular-nums">{{ hms(currentTime) }}</span>
            <span
              class="px-2 py-1 rounded-full border border-slate-700 bg-slate-900 text-slate-200 text-sm select-none transition-opacity"
              :class="{ 'opacity-60': !hasPDT }" tabindex="0">
              {{ pdtLabel }}
            </span>

            <div class="flex-1"></div>

            <!-- Jump to datetime -->
            <div
              class="flex items-center gap-2 px-2 py-1 rounded-xl border border-slate-800 bg-slate-900 text-slate-400">
              <label for="dt" class="text-slate-400">Springe zu:</label>
              <input id="dt" v-model="dtValue" type="datetime-local" step="1"
                class="bg-transparent outline-none text-slate-100" />
              <button @click="onJump"
                class="px-2 py-1 rounded-full bg-slate-800 text-slate-100 font-semibold hover:brightness-110">
                <LucideCalendarArrowUp class="size-4" />
              </button>
            </div>

            <button @click="toggleFS"
              class="px-3 py-2 rounded-full bg-slate-800 text-slate-100 font-semibold hover:brightness-110"
              title="Vollbild">
              <LucideFullscreen class="size-4" />
            </button>
          </div>
        </div>
      </div>
    </div>
  </ClientOnly>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref, computed, nextTick, watch, watchEffect } from 'vue'
import Hls, { Events as HlsEvents, ErrorTypes as HlsErrorTypes, type LevelDetails } from 'hls.js'
import { ClientOnly } from '#components'
import { LucideCalendarArrowUp, LucideFullscreen, LucidePause, LucidePlay, LucideVolume2, LucideVolumeOff } from 'lucide-vue-next'

/** CONFIG **/
const src: string = 'http://192.168.178.124:9901/vod/ef29-summerboat/index.m3u8' // <— anpassen
const LAST_SEG_SAFE_DELTA = 0.25 // s
const REFRESH_NATIVE_MS = 4000   // ms
const CONTROLS_HIDE_MS = 2000    // ms

/** REFS / STATE **/
const videoEl = ref<HTMLVideoElement | null>(null)
const playerEl = ref<HTMLDivElement | null>(null)
let hls: Hls | null = null
let raf = 0
let hideTimer: number | null = null

const isPlaying = ref(false)
const muted = ref(false)
const volume = ref(1)
const prevVolume = ref(0.6)
const controlsVisible = ref(true)
const userInteracting = ref(false)

const playbackRate = ref(1)
const speedLabel = computed(() => `${Number(playbackRate.value.toFixed(2))}×`)
const dtValue = ref<string>('')

const currentTime = ref(0)
const bufferedEnd = ref(0)
const lastDetails = ref<LevelDetails | null>(null)
const playlistType = ref<'VOD' | 'EVENT' | null>(null)

/** Native HLS parsed frags **/
interface FragInfo { start: number; end: number; duration: number; programDateTime?: number }
const manualFrags = ref<FragInfo[]>([])
let manualTimer: number | null = null

/** FIX: Live-Sync als ref + watchEffect **/
const liveSync = ref(0)
watchEffect(() => {
  // Reaktive Abhängigkeiten, damit das Effect auf RAF/Buffer-Updates reagiert
  const _tick = bufferedEnd.value || currentTime.value
  const pos = (hls && typeof (hls as any).liveSyncPosition === 'number')
    ? (hls as any).liveSyncPosition as number
    : _tick
  liveSync.value = pos
})

/** COMPUTED **/
const liveDelta = computed(() => liveSync.value - currentTime.value) // + = hinter Live, - = vor Live
const atLiveEdge = computed(() => Math.abs(liveDelta.value) < 0.75)  // gleicher Schwellwert überall
const liveLag = computed(() => Math.max(0, liveDelta.value))
const isEvent = computed(() => playlistType.value !== 'VOD')

const playedPct = computed<number>(() => {
  const [ws, we] = dvrWindow()
  if (we <= ws) return 0
  return Math.max(0, Math.min(100, ((currentTime.value - ws) / (we - ws)) * 100))
})
const bufferedPct = computed<number>(() => {
  const [ws, we] = dvrWindow()
  if (we <= ws) return 0
  return Math.max(0, Math.min(100, ((bufferedEnd.value - ws) / (we - ws)) * 100))
})

/** LIFECYCLE **/
onMounted(() => {
  nextTick(() => {
    setupVideo()
    setupPlayback()
    setupRAF()
    seedDatetime()
  })
})

onUnmounted(() => {
  cancelAnimationFrame(raf)
  try { hls?.destroy() } catch { }
  if (manualTimer != null) clearTimeout(manualTimer)
  if (hideTimer != null) clearTimeout(hideTimer)
})

/** SETUP **/
function setupVideo() {
  const v = videoEl.value
  if (!v) return

  v.addEventListener('play', () => { isPlaying.value = true; scheduleHide() })
  v.addEventListener('pause', () => { isPlaying.value = false; showControls() })
  v.addEventListener('ratechange', () => playbackRate.value = v.playbackRate)
  v.addEventListener('volumechange', () => {
    volume.value = v.volume
    muted.value = v.muted || v.volume === 0
  })

  // init
  volume.value = v.volume ?? 1
  muted.value = v.muted ?? false
}

function scheduleHide() {
  if (hideTimer != null) window.clearTimeout(hideTimer)
  if (!isPlaying.value || userInteracting.value) { showControls(); return }
  hideTimer = window.setTimeout(() => { controlsVisible.value = false }, CONTROLS_HIDE_MS)
}
function showControls() {
  controlsVisible.value = true
}

function setupPlayback() {
  const v = videoEl.value
  if (!v) return

  if (Hls.isSupported()) {
    hls = new Hls({ lowLatencyMode: true, liveSyncDurationCount: 3, backBufferLength: 90, enableWorker: true })

    hls.on(HlsEvents.ERROR, (_e, data: any) => {
      if (!data?.fatal) return
      switch (data.type) {
        case HlsErrorTypes.NETWORK_ERROR: try { hls!.startLoad() } catch { } break
        case HlsErrorTypes.MEDIA_ERROR: try { hls!.recoverMediaError() } catch { } break
        default:
          try { hls?.destroy() } catch { }
          hls = null
          setupPlayback()
          return
      }
    })

      hls.on(HlsEvents.LEVEL_UPDATED, (_e, { details }) => {
        lastDetails.value = details
        playlistType.value = details.type as 'VOD' | 'EVENT' | null
      })

    hls.attachMedia(v)
    hls.loadSource(src)
  } else if (v.canPlayType('application/vnd.apple.mpegurl')) {
    v.src = src
    scheduleManualPlaylistRefresh()
  } else {
    alert('HLS wird in diesem Browser nicht unterstützt.')
  }
}

/** MANUAL M3U8 PARSE (für native HLS) **/
function scheduleManualPlaylistRefresh() {
  if (manualTimer != null) return
  const loop = async () => {
    try {
      const res = await fetch(src, { cache: 'no-store' })
      const txt = await res.text()
      const { frags, type } = parseM3U8(txt)
      manualFrags.value = frags
      playlistType.value = type
    } catch { }
    manualTimer = window.setTimeout(loop, REFRESH_NATIVE_MS)
  }
  loop()
}

function parseM3U8(text: string): { frags: FragInfo[]; seq: number; type: 'VOD' | 'EVENT' | null } {
  const lines = text.split(/\r?\n/)
  let seq = 0
  let targetDuration = 0
  let curDur: number | null = null
  let curPDT: number | undefined
  let type: 'VOD' | 'EVENT' | null = null
  const frags: FragInfo[] = []
  const afterColon = (s: string) => s.split(':')[1] ?? ''

  for (const ln of lines) {
    if (ln.startsWith('#EXT-X-MEDIA-SEQUENCE:')) seq = parseInt(afterColon(ln) || '0', 10)
    else if (ln.startsWith('#EXT-X-TARGETDURATION:')) targetDuration = parseFloat(afterColon(ln) || '0')
    else if (ln.startsWith('#EXT-X-PLAYLIST-TYPE:')) {
      const val = afterColon(ln).trim()
      if (val === 'VOD') type = 'VOD'
      else if (val === 'EVENT') type = 'EVENT'
    }
    else if (ln.startsWith('#EXTINF:')) curDur = parseFloat(afterColon(ln) || `${targetDuration}`)
    else if (ln.startsWith('#EXT-X-PROGRAM-DATE-TIME:')) {
      const val = ln.substring('#EXT-X-PROGRAM-DATE-TIME:'.length).trim()
      const t = Date.parse(val)
      if (!isNaN(t)) curPDT = t
    }
    else if (ln && !ln.startsWith('#')) {
      const dur = curDur ?? targetDuration
      const start = frags.length ? frags[frags.length - 1]!.end : 0
      const end = start + dur
      frags.push({ start, end, duration: dur, programDateTime: curPDT })
      curDur = null
      curPDT = undefined
    }
  }
  return { frags, seq, type }
}

/** RAF **/
function setupRAF() {
  const v = videoEl.value
  if (!v) {
    raf = requestAnimationFrame(setupRAF)
    return
  }

  const tick = () => {
    try {
      if (videoEl.value) {
        currentTime.value = videoEl.value.currentTime
        const b = videoEl.value.buffered
        bufferedEnd.value = b.length ? b.end(b.length - 1) : videoEl.value.currentTime
      }
    } catch { }
    raf = requestAnimationFrame(tick)
  }
  raf = requestAnimationFrame(tick)
}

/** DVR WINDOW **/
function dvrWindow(): [number, number] {
  const det = lastDetails.value
  const frags = (det?.fragments ?? []) as Array<any>
  if (frags.length) {
    const first = frags[0]!
    const last = frags[frags.length - 1]!
    const ws = first.start as number
    const we = ((last.end as number | undefined) ?? (det!.totalduration as number | undefined) ?? bufferedEnd.value)
    return [ws, we]
  }
  if (manualFrags.value.length) {
    return [manualFrags.value[0]!.start, manualFrags.value[manualFrags.value.length - 1]!.end]
  }
  const v = videoEl.value
  if (!v) return [0, 0]

  const b = v.buffered
  if (b.length) return [b.start(0), b.end(b.length - 1)]
  return [0, Math.max(bufferedEnd.value, currentTime.value)]
}

/** Controls **/
function togglePlay() {
  const v = videoEl.value
  if (!v) return
  if (v.paused) v.play().catch(() => { })
  else v.pause()
}
function toggleMute() {
  const v = videoEl.value
  if (!v) return
  if (v.muted || v.volume === 0) {
    v.muted = false
    v.volume = prevVolume.value || 0.6
  } else {
    prevVolume.value = v.volume
    v.muted = true
  }
}
function onVolumeInput() {
  const v = videoEl.value
  if (!v) return
  const val = Math.min(1, Math.max(0, volume.value))
  v.volume = val
  if (val === 0) v.muted = true
  else if (v.muted) v.muted = false
}

function goLive() {
  const v = videoEl.value
  if (!v) return
  v.currentTime = liveSync.value
  v.play().catch(() => { })
  hls?.startLoad?.()
}
function cycleSpeed() {
  const v = videoEl.value
  if (!v) return
  const rates = [1, 1.25, 1.5, 1.75, 2]
  const idx = (rates.indexOf(v.playbackRate) + 1) % rates.length
  v.playbackRate = rates[idx]!
}
async function togglePiP() {
  const v = videoEl.value
  if (!v) return
  try {
    // @ts-ignore
    if (document.pictureInPictureElement) await document.exitPictureInPicture()
    // @ts-ignore
    else if (document.pictureInPictureEnabled && !v.disablePictureInPicture) await v.requestPictureInPicture()
  } catch { }
}
function toggleFS() {
  const el = playerEl.value!
  if (!document.fullscreenElement) el.requestFullscreen?.().catch(() => { })
  else document.exitFullscreen?.().catch(() => { })
}

/** Helpers & PDT **/
function toDatetimeLocalString(ms: number) {
  const d = new Date(ms)
  d.setMinutes(d.getMinutes() - d.getTimezoneOffset())
  return d.toISOString().slice(0, 19) // "YYYY-MM-DDTHH:mm:ss"
}

// helper: gibt es überhaupt PDT?
const hasPDT = computed<boolean>(() => {
  const det = lastDetails.value
  const frags = (det?.fragments ?? []) as any[]
  const anyHls = frags.some(f => f?.programDateTime != null)
  const anyNative = manualFrags.value.some(f => f.programDateTime != null)
  return anyHls || anyNative
})

/** DVR-PDT-Grenzen (für Eingabe-Min/Max und Clamping) **/
const dvrPdtBounds = computed<{ min: number, max: number } | null>(() => {
  // Hls.js
  const det = lastDetails.value
  const frags = (det?.fragments ?? []) as any[]
  const withPdt = frags.filter(f => f?.programDateTime != null)
  if (withPdt.length) {
    const min = Number(withPdt[0].programDateTime)
    const last = withPdt[withPdt.length - 1]
    const max = Number(last.programDateTime) + Number(last.duration ?? 0) * 1000
    return { min, max }
  }
  // Native Fallback
  const mf = manualFrags.value.filter(f => f.programDateTime != null)
  if (mf.length) {
    const min = mf[0]!.programDateTime as number
    const last = mf[mf.length - 1]!
    const max = (last.programDateTime as number) + last.duration * 1000
    return { min, max }
  }
  return null
})

// Optional: direkt verwendbare Strings für <input :min/:max>
const dtMinStr = computed<string | undefined>(() => dvrPdtBounds.value ? toDatetimeLocalString(dvrPdtBounds.value.min) : undefined)
const dtMaxStr = computed<string | undefined>(() => dvrPdtBounds.value ? toDatetimeLocalString(dvrPdtBounds.value.max) : undefined)

/** PDT <-> Medienzeit **/
function pdtForMediaTime(t: number): number | null {
  const det = lastDetails.value
  const frags = (det?.fragments ?? []) as any[]
  const has = frags.some(f => f?.programDateTime != null)
  if (frags.length && has) {
    let lo = 0, hi = frags.length - 1, idx = 0
    while (lo <= hi) { const mid = (lo + hi) >> 1; const s = Number(frags[mid].start ?? 0); if (s <= t) { idx = mid; lo = mid + 1 } else { hi = mid - 1 } }
    let base = idx; while (base >= 0 && frags[base].programDateTime == null) base--
    const baseFrag = base >= 0 ? frags[base] : frags.find((f: any) => f?.programDateTime != null)!
    const baseMedia = Number(baseFrag.start ?? 0)
    const basePdt = Number(baseFrag.programDateTime)
    return basePdt + Math.max(0, t - baseMedia) * 1000
  }
  if (manualFrags.value.length) {
    const mf = manualFrags.value
    if (mf.some(f => f.programDateTime != null)) {
      let lo = 0, hi = mf.length - 1, idx = 0
      while (lo <= hi) { const mid = (lo + hi) >> 1; const s = mf[mid]!.start; if (s <= t) { idx = mid; lo = mid + 1 } else { hi = mid - 1 } }
      let base = idx; while (base >= 0 && mf[base]!.programDateTime == null) base--
      const baseFrag = base >= 0 ? mf[base]! : mf.find(f => f.programDateTime != null)!
      const baseMedia = base >= 0 ? mf[base]!.start : baseFrag.start
      const basePdt = baseFrag.programDateTime as number
      return basePdt + Math.max(0, t - baseMedia) * 1000
    }
  }
  return null
}

function formatDateTimeDE(d: Date): string {
  const pad = (n: number, len = 2) => String(n).padStart(len, '0')
  return `${pad(d.getDate())}.${pad(d.getMonth() + 1)}.${d.getFullYear()} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`
}

const recordingPdtMs = computed<number | null>(() => pdtForMediaTime(currentTime.value))
const lagLabel = computed(() =>
  liveDelta.value >= 0
    ? `+${liveDelta.value.toFixed(1)}s hinter Live`
    : `${Math.abs(liveDelta.value).toFixed(1)}s vor Live`
)
const pdtLabel = computed(() => (recordingPdtMs.value != null ? formatDateTimeDE(new Date(recordingPdtMs.value)) : 'Zeitstempel n/a'))

/** Jump (PDT) **/
function onJump() {
  if (!dtValue.value) return
  const parsed = Date.parse(dtValue.value)
  if (isNaN(parsed)) {
    alert('Ungültiges Datumsformat')
    return
  }

  const bounds = dvrPdtBounds.value
  if (!bounds) {
    alert('Diese Aufzeichnung enthält keine Zeitstempel-Informationen')
    return
  }

  // ggf. bestätigen, wenn außerhalb – und zum nächsten Rand snappen
  let targetMs = parsed
  if (targetMs < bounds.min || targetMs > bounds.max) {
    const choice = confirm(
      `Ausgewählter Zeitpunkt ist außerhalb des verfügbaren Fensters:\n` +
      `Von ${formatDateTimeDE(new Date(bounds.min))}\n` +
      `Bis ${formatDateTimeDE(new Date(bounds.max))}\n\n` +
      `Zum nächstliegenden Punkt springen?`
    )
    if (!choice) return
    targetMs = Math.min(Math.max(targetMs, bounds.min), bounds.max)
  }

  let t = currentTime.value
  let foundMatch = false

  // Hls.js vorrangig, wenn PDT vorhanden
  const det = lastDetails.value
  const frags = (det?.fragments ?? []).filter((f: any) => f?.programDateTime != null) as any[]

  if (frags.length) {
    let lo = 0, hi = frags.length - 1, idx = 0
    while (lo <= hi) {
      const mid = (lo + hi) >> 1
      const pdt = Number(frags[mid].programDateTime)
      if (pdt <= targetMs) { idx = mid; lo = mid + 1 } else { hi = mid - 1 }
    }
    const f = frags[idx]
    if (f) {
      const offsetS = Math.max(0, (targetMs - Number(f.programDateTime)) / 1000)
      const fragDur = Number.isFinite(f.duration) ? Number(f.duration) : 0
      t = Number(f.start) + Math.min(offsetS, fragDur)

      const all = det!.fragments as any[]
      const minStart = Number(all[0]!.start ?? 0)
      const lastFrag = all[all.length - 1]!
      const maxEnd = Number((lastFrag.end as number | undefined) ?? (det!.totalduration as number | undefined) ?? t)
      t = Math.min(Math.max(t, minStart), maxEnd)
      foundMatch = true
    }
  } else {
    // Native Fallback
    const mf = manualFrags.value.filter(f => f.programDateTime != null)
    if (mf.length) {
      let lo = 0, hi = mf.length - 1, idx = 0
      while (lo <= hi) {
        const mid = (lo + hi) >> 1
        const pdt = mf[mid]!.programDateTime as number
        if (pdt <= targetMs) { idx = mid; lo = mid + 1 } else { hi = mid - 1 }
      }
      const f = mf[idx]
      if (f) {
        const offsetS = Math.max(0, (targetMs - (f.programDateTime as number)) / 1000)
        t = f.start + Math.min(offsetS, f.duration)
        const minStart = mf[0]!.start
        const maxEnd = mf[mf.length - 1]!.end
        t = Math.min(Math.max(t, minStart), maxEnd)
        foundMatch = true
      }
    }
  }

  if (!foundMatch) {
    alert('Der ausgewählte Zeitpunkt ist in der Aufzeichnung nicht verfügbar')
    return
  }

  const v = videoEl.value
  if (!v) return
  seekSafely(t)
  v.play?.().catch(() => { })
}

/** Seekbar **/
function onBarPointer(e: PointerEvent) {
  userInteracting.value = true
  if (e.pointerType === 'mouse') showControls()
  const el = e.currentTarget as HTMLDivElement
  const rect = el.getBoundingClientRect()
  const getP = (clientX: number) => Math.max(0, Math.min(1, (clientX - rect.left) / rect.width))
  seekToPct(getP(e.clientX))
  const move = (ev: PointerEvent) => {
    seekToPct(getP(ev.clientX))
  }
  const up = () => {
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', up)
    userInteracting.value = false
    scheduleHide()
  }
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
}
function seekToPct(p: number) {
  const [ws, we] = dvrWindow()
  const t = ws + p * (we - ws)
  const v = videoEl.value
  if (!v) return
  v.currentTime = t
  hls?.startLoad?.()
}

function seekSafely(target: number) {
  const v = videoEl.value
  if (!v) return
  const b = v.buffered
  let desired = target
  let inside = false
  for (let i = 0; i < b.length; i++) {
    const s = b.start(i), e = b.end(i)
    if (target >= s && target <= e) {
      inside = true
      desired = Math.min(target, Math.max(e - LAST_SEG_SAFE_DELTA, s))
      break
    }
  }
  v.currentTime = inside ? desired : target
}

/** Controls visibility interactions **/
function onMouseMove() {
  showControls()
  scheduleHide()
}
function onMouseLeave() {
  if (isPlaying.value) controlsVisible.value = false
}
function onKeydown(e: KeyboardEvent) {
  if ([" ", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) {
    showControls()
    scheduleHide()
  }
}

/** Seed datetime input **/
function seedDatetime() {
  // Startwert: "jetzt -5s" (lokal). Wird unten per Watch ggf. ins DVR-Fenster gezogen.
  const now = Date.now() - 5000
  dtValue.value = toDatetimeLocalString(now)
}
/** Format seconds as HH:MM:SS **/
function hms(sec: number | { value: number }): string {
  const s = typeof sec === 'number' ? sec : sec.value
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const ss = Math.floor(s % 60)
  return [h, m, ss].map(v => v.toString().padStart(2, '0')).join(':')
}

// Wenn die DVR-Grenzen erstmals bekannt sind oder sich ändern:
// Falls die aktuelle Eingabe außerhalb liegt (oder leer), in den gültigen Bereich ziehen.
watch(dvrPdtBounds, (b) => {
  if (!b) return
  const cur = Date.parse(dtValue.value)
  if (!dtValue.value || isNaN(cur) || cur < b.min || cur > b.max) {
    // möglichst nahe am Ende, aber noch im Fenster
    const seed = Math.min(Math.max(Date.now() - 5000, b.min), b.max)
    dtValue.value = toDatetimeLocalString(seed)
  }
})
</script>
