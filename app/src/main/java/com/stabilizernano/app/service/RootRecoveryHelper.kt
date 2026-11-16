package com.stabilizernano.app.service

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.TimeUnit

object RootRecoveryHelper {
    private const val RECOVERY_CMD = "svc data disable && sleep 1 && svc data enable"
    private const val TIMEOUT_MS = 12_000L

    suspend fun toggleMobileDataAsRoot(): Result<String> = withContext(Dispatchers.IO) {
        runCatching {
            val process = ProcessBuilder("su", "-c", RECOVERY_CMD)
                .redirectErrorStream(true)
                .start()

            val completed = withTimeoutOrNull(TIMEOUT_MS) {
                process.waitFor(TIMEOUT_MS, TimeUnit.MILLISECONDS)
            }

            if (completed != true) {
                process.destroyForcibly()
                error("Root recovery timed out after ${TIMEOUT_MS}ms")
            }

            val output = process.inputStream.bufferedReader().use { it.readText().trim() }
            val exitCode = process.exitValue()
            if (exitCode != 0) {
                val detail = output.ifEmpty { "exit code $exitCode" }
                error("su failed: $detail")
            }
            "Cellular radio toggled via su ($RECOVERY_CMD)"
        }
    }
}
