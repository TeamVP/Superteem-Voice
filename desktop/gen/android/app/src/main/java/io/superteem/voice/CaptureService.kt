package io.superteem.voice

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.bluetooth.BluetoothManager
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.ParcelUuid
import android.os.SystemClock
import java.io.File
import java.util.UUID

private val PIN_SERVICE: UUID = UUID.fromString("5354564f-1000-4e00-8000-535456000001")
private const val MAX_PINS = 3
private const val CHANNEL_ID = "capture"
private const val NOTIFICATION_ID = 1
private const val JOIN_TIMEOUT_MS = 3000L

const val INPUT_PHONE = "Phone microphone"
const val INPUT_HEADSET = "Bluetooth headset"

/**
 * Owns recording. Runs apart from the UI, so it continues with the screen locked, and writes one
 * journal per stream into the session folder the app chose.
 */
@SuppressLint("MissingPermission")
class CaptureService : Service() {
    private val pins = LinkedHashMap<String, PinSource>()
    private val mics = mutableListOf<MicSource>()
    private lateinit var sessionDir: File
    private var scanning = false

    private val scanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            synchronized(pins) {
                val address = result.device.address
                if (pins.size >= MAX_PINS || address in pins || !running) return
                val source = PinSource(result.device, sessionDir, "pin${pins.size + 1}")
                pins[address] = source
                source.start()
                if (pins.size >= MAX_PINS) stopScan()
            }
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Recording", NotificationManager.IMPORTANCE_LOW),
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (running || intent == null) return START_NOT_STICKY
        val inputs = intent.getStringArrayExtra(EXTRA_INPUTS).orEmpty()
        val usesPins = intent.getBooleanExtra(EXTRA_PINS, false)
        startInForeground(bluetooth = usesPins || INPUT_HEADSET in inputs)
        running = true
        instance = this
        startedAt = SystemClock.elapsedRealtime()
        Stats.sources.clear()
        sessionDir = File(intent.getStringExtra(EXTRA_DIR)!!).apply { mkdirs() }
        for (input in inputs) {
            val source = when (input) {
                INPUT_HEADSET -> MicSource(this, sessionDir, "headset", headset = true)
                INPUT_PHONE -> MicSource(this, sessionDir, "phone", headset = false)
                else -> null
            }
            source?.let { mics += it; it.start() }
        }
        if (usesPins) startScan()
        return START_NOT_STICKY
    }

    /** The connected-device type needs Bluetooth permission, so it is claimed only when Bluetooth is used. */
    private fun startInForeground(bluetooth: Boolean) {
        val notification = Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("Voice is recording")
            .setContentText("Recording audio streams")
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            if (bluetooth) types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
            startForeground(NOTIFICATION_ID, notification, types)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun scanner() = getSystemService(BluetoothManager::class.java).adapter?.bluetoothLeScanner

    private fun startScan() {
        val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(PIN_SERVICE)).build()
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        scanner()?.startScan(listOf(filter), settings, scanCallback)
        scanning = true
    }

    private fun stopScan() {
        if (scanning) scanner()?.stopScan(scanCallback)
        scanning = false
    }

    /** Stops every stream and waits for its journal to close, so processing sees complete files. */
    fun finish() {
        running = false
        stopScan()
        val sources: List<Thread> = synchronized(pins) { pins.values.toList() } + mics
        pins.values.forEach { it.shutdown() }
        mics.forEach { it.shutdown() }
        sources.forEach { it.join(JOIN_TIMEOUT_MS) }
    }

    override fun onDestroy() {
        if (running) finish()
        instance = null
        super.onDestroy()
    }

    companion object {
        const val EXTRA_DIR = "dir"
        const val EXTRA_INPUTS = "inputs"
        const val EXTRA_PINS = "pins"

        @Volatile
        var running = false
            private set

        @Volatile
        var instance: CaptureService? = null
            private set

        @Volatile
        var startedAt = 0L
            private set
    }
}
