package io.superteem.voice

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.SystemClock
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class StartArgs {
    var dir: String = ""
    var inputs: Array<String> = arrayOf()
    var pins: Boolean = false
}

@InvokeArg
class ProbeArgs {
    var inputs: Array<String> = arrayOf()
}

@InvokeArg
class UrlArgs {
    var url: String = ""
}

@InvokeArg
class SpeakArgs {
    var text: String = ""
    var rate: Float = 1f
    var pitch: Float = 1f
}

@InvokeArg
class PermissionArgs {
    var ids: Array<String> = arrayOf()
}

/** The bridge between the shared app (src/recorder.rs) and CaptureService. */
@TauriPlugin
class CapturePlugin(private val activity: Activity) : Plugin(activity) {
    /** The access the app asks for, by the name the page uses, with the Android permissions behind each. */
    private fun groups(): Map<String, List<String>> {
        val map = linkedMapOf("microphone" to listOf(Manifest.permission.RECORD_AUDIO))
        map["nearby"] = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            listOf(Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.BLUETOOTH_SCAN)
        } else {
            listOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            map["notifications"] = listOf(Manifest.permission.POST_NOTIFICATIONS)
        }
        return map
    }

    private fun granted(permission: String) = activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    private val asked get() = activity.getSharedPreferences("permissions", Activity.MODE_PRIVATE)

    /** Recording needs the microphone, and Bluetooth for a headset or pins. Notifications are optional. */
    private fun missingPermissions(pins: Boolean, headset: Boolean): List<String> {
        val needed = mutableListOf(Manifest.permission.RECORD_AUDIO)
        if (pins || headset) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                needed += Manifest.permission.BLUETOOTH_CONNECT
                if (pins) needed += Manifest.permission.BLUETOOTH_SCAN
            } else if (pins) {
                needed += Manifest.permission.ACCESS_FINE_LOCATION
            }
        }
        return needed.filter { !granted(it) }
    }

    /**
     * Each kind of access: whether it is allowed, and whether Android will still ask. Once someone has said
     * no and Android stops showing the question, it can only be allowed in the app's Android settings.
     */
    @Command
    fun permissions(invoke: Invoke) {
        val list = JSArray()
        for ((id, perms) in groups()) {
            val allowed = perms.all { granted(it) }
            val wasAsked = perms.any { asked.getBoolean(it, false) }
            val canAsk = !wasAsked || perms.any { !granted(it) && activity.shouldShowRequestPermissionRationale(it) }
            val item = JSObject()
            item.put("id", id)
            item.put("allowed", allowed)
            item.put("blocked", !allowed && !canAsk)
            list.put(item)
        }
        val result = JSObject()
        result.put("permissions", list)
        invoke.resolve(result)
    }

    /** Shows Android's question for each kind of access asked for. The page checks again once it returns. */
    @Command
    fun askPermissions(invoke: Invoke) {
        val args = invoke.parseArgs(PermissionArgs::class.java)
        val all = groups()
        val perms = args.ids.flatMap { all[it].orEmpty() }.filter { !granted(it) }
        if (perms.isNotEmpty()) {
            asked.edit().apply { perms.forEach { putBoolean(it, true) } }.apply()
            activity.requestPermissions(perms.toTypedArray(), 2)
        }
        invoke.resolve(JSObject())
    }

    /** Opens a web page in the browser, such as the download for a newer version. */
    @Command
    fun openUrl(invoke: Invoke) {
        val args = invoke.parseArgs(UrlArgs::class.java)
        try {
            activity.startActivity(Intent(Intent.ACTION_VIEW, android.net.Uri.parse(args.url)))
            invoke.resolve(JSObject())
        } catch (e: Exception) {
            invoke.reject("No browser could open ${args.url}")
        }
    }

    // Reading text aloud with the phone's own voice, as Android's web view has none.
    private var tts: android.speech.tts.TextToSpeech? = null
    private var ttsReady = false
    private val speaking = mutableMapOf<String, Invoke>()

    private fun withTts(then: (android.speech.tts.TextToSpeech?) -> Unit) {
        tts?.let { if (ttsReady) return then(it) }
        tts = android.speech.tts.TextToSpeech(activity) { status ->
            ttsReady = status == android.speech.tts.TextToSpeech.SUCCESS
            tts?.setOnUtteranceProgressListener(object : android.speech.tts.UtteranceProgressListener() {
                override fun onStart(id: String) {}
                override fun onDone(id: String) { speaking.remove(id)?.resolve(JSObject()) }
                @Deprecated("Deprecated in Java")
                override fun onError(id: String) { speaking.remove(id)?.resolve(JSObject()) }
                override fun onStop(id: String, interrupted: Boolean) { speaking.remove(id)?.resolve(JSObject()) }
            })
            then(if (ttsReady) tts else null)
        }
    }

    /** Says the text, answering once it has been said or stopped. */
    @Command
    fun speak(invoke: Invoke) {
        val args = invoke.parseArgs(SpeakArgs::class.java)
        withTts { engine ->
            if (engine == null) return@withTts invoke.reject("This phone has no text-to-speech voice.")
            val id = "u" + SystemClock.elapsedRealtimeNanos()
            speaking[id] = invoke
            engine.setSpeechRate(args.rate.coerceIn(0.5f, 2f))
            engine.setPitch(args.pitch.coerceIn(0.5f, 2f))
            engine.speak(args.text, android.speech.tts.TextToSpeech.QUEUE_ADD, null, id)
        }
    }

    @Command
    fun stopSpeaking(invoke: Invoke) {
        tts?.stop()
        speaking.values.forEach { it.resolve(JSObject()) }
        speaking.clear()
        invoke.resolve(JSObject())
    }

    /** A short buzz, to say dictation stopped by itself. */
    @Command
    fun vibrate(invoke: Invoke) {
        val effect = android.os.VibrationEffect.createOneShot(120, android.os.VibrationEffect.DEFAULT_AMPLITUDE)
        if (Build.VERSION.SDK_INT >= 31) {
            val manager = activity.getSystemService(android.os.VibratorManager::class.java)
            manager?.defaultVibrator?.vibrate(effect)
        } else {
            @Suppress("DEPRECATION")
            (activity.getSystemService(android.content.Context.VIBRATOR_SERVICE) as? android.os.Vibrator)?.vibrate(effect)
        }
        invoke.resolve(JSObject())
    }

    /** The app's page in Android settings, where access refused for good can be allowed. */
    @Command
    fun openAppSettings(invoke: Invoke) {
        val intent = Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS)
            .setData(android.net.Uri.fromParts("package", activity.packageName, null))
        activity.startActivity(intent)
        invoke.resolve(JSObject())
    }

    @Command
    fun inputs(invoke: Invoke) {
        val result = JSObject()
        result.put("devices", JSArray(listOf(INPUT_PHONE, INPUT_HEADSET)))
        result.put("default", INPUT_PHONE)
        invoke.resolve(result)
    }

    /** Microphones listened to for their meters on the New meeting page, by the names `inputs` gives. */
    private val probes = mutableMapOf<String, MicSource>()

    private fun stopProbes() {
        probes.values.forEach { it.shutdown() }
        probes.values.forEach { runCatching { it.join(1000) } }
        probes.clear()
        ProbeStats.sources.clear()
    }

    /** Listens to each microphone without recording, so its meter moves. Ones not yet allowed are left alone. */
    @Command
    fun probeStart(invoke: Invoke) {
        val args = invoke.parseArgs(ProbeArgs::class.java)
        stopProbes()
        if (!CaptureService.running) {
            for (name in args.inputs) {
                if (missingPermissions(false, name == INPUT_HEADSET).isNotEmpty()) continue
                val source = when (name) {
                    INPUT_HEADSET -> MicSource(activity, null, name, headset = true, stats = ProbeStats)
                    INPUT_PHONE -> MicSource(activity, null, name, headset = false, stats = ProbeStats)
                    else -> continue
                }
                probes[name] = source
                source.start()
            }
        }
        invoke.resolve(JSObject())
    }

    @Command
    fun probeStatus(invoke: Invoke) {
        val list = JSArray()
        for ((name, s) in ProbeStats.sources.toSortedMap()) {
            val probe = JSObject()
            probe.put("name", name)
            probe.put("state", s.state)
            probe.put("detail", s.detail)
            probe.put("level", s.level.toDouble())
            list.put(probe)
        }
        val result = JSObject()
        result.put("probes", list)
        invoke.resolve(result)
    }

    @Command
    fun probeStop(invoke: Invoke) {
        Thread {
            stopProbes()
            invoke.resolve(JSObject())
        }.start()
    }

    @Command
    fun start(invoke: Invoke) {
        val args = invoke.parseArgs(StartArgs::class.java)
        if (CaptureService.running) return invoke.reject("Already recording.")
        // The microphones are let go by the meters before the recording opens them.
        stopProbes()
        val missing = missingPermissions(args.pins, INPUT_HEADSET in args.inputs)
        if (missing.isNotEmpty()) {
            asked.edit().apply { missing.forEach { putBoolean(it, true) } }.apply()
            activity.requestPermissions(missing.toTypedArray(), 1)
            return invoke.reject("Voice needs access to the microphone. Allow it, then try again.")
        }
        val intent = Intent(activity, CaptureService::class.java)
            .putExtra(CaptureService.EXTRA_DIR, args.dir)
            .putExtra(CaptureService.EXTRA_INPUTS, args.inputs)
            .putExtra(CaptureService.EXTRA_PINS, args.pins)
        activity.startForegroundService(intent)
        invoke.resolve(JSObject())
    }

    @Command
    fun status(invoke: Invoke) {
        val result = JSObject()
        val started = CaptureService.startedAt
        result.put("elapsed_s", if (started > 0) (SystemClock.elapsedRealtime() - started) / 1000 else 0)
        val streams = JSArray()
        for ((name, s) in Stats.sources.toSortedMap()) {
            val stream = JSObject()
            stream.put("name", name)
            stream.put("state", s.state)
            stream.put("detail", if (s.reconnects > 0) "${s.detail} (reconnected ${s.reconnects}×)".trim() else s.detail)
            stream.put("frames", s.frames)
            stream.put("lost", s.lost)
            stream.put("level", s.level.toDouble())
            streams.put(stream)
        }
        result.put("streams", streams)
        invoke.resolve(result)
    }

    /** Resolves only after every journal is closed, off the main thread. */
    @Command
    fun stop(invoke: Invoke) {
        Thread {
            CaptureService.instance?.finish()
            activity.runOnUiThread {
                activity.stopService(Intent(activity, CaptureService::class.java))
                invoke.resolve(JSObject())
            }
        }.start()
    }
}
