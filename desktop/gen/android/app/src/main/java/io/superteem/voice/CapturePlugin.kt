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

    @Command
    fun start(invoke: Invoke) {
        val args = invoke.parseArgs(StartArgs::class.java)
        if (CaptureService.running) return invoke.reject("Already recording.")
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
