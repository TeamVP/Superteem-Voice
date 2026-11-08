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

/** The bridge between the shared app (src/recorder.rs) and CaptureService. */
@TauriPlugin
class CapturePlugin(private val activity: Activity) : Plugin(activity) {
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
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            needed += Manifest.permission.POST_NOTIFICATIONS
        }
        return needed.filter { activity.checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
    }

    @Command
    fun inputs(invoke: Invoke) {
        val result = JSObject()
        result.put("devices", JSArray(listOf(INPUT_PHONE, INPUT_HEADSET)))
        result.put("default", INPUT_PHONE)
        invoke.resolve(result)
    }

    @Command
    fun start(invoke: Invoke) {
        val args = invoke.parseArgs(StartArgs::class.java)
        if (CaptureService.running) return invoke.reject("Already recording.")
        val missing = missingPermissions(args.pins, INPUT_HEADSET in args.inputs)
        if (missing.isNotEmpty()) {
            activity.requestPermissions(missing.toTypedArray(), 1)
            return invoke.reject("Allow the requested access, then press Start recording again.")
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
