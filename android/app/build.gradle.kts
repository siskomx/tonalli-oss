import java.nio.file.Files
import java.security.MessageDigest

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val branding = groovy.json.JsonSlurper().parse(rootProject.file("../branding.json")) as Map<*, *>
val aiEnabled = providers.gradleProperty("enableAi").map { it.toBooleanStrict() }.getOrElse(false)
val meshEnabled = providers.gradleProperty("enableMesh").map { it.toBooleanStrict() }.getOrElse(false)
val coreGeneratedRoot = if (meshEnabled) { if (aiEnabled) "generated/ai-mesh" else "generated/mesh" } else if (aiEnabled) "generated/ai" else "generated"

android {
    namespace = "org.tonalli.spike"
    compileSdk = 34
    buildToolsVersion = "35.0.0"
    ndkVersion = "29.0.14206865"
    defaultConfig {
        applicationId = "org.tonalli.spike"
        // Temporary plan assumption until the actual phone Android version is reported.
        minSdk = 34
        targetSdk = 34
        versionCode = 1
        versionName = "0.1-m1-spike"
        resValue("string", "app_name", "${branding["displayName"]}")
        ndk { abiFilters += "arm64-v8a" }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    sourceSets["main"].apply {
        java.srcDir(layout.buildDirectory.dir("$coreGeneratedRoot/uniffi"))
        jniLibs.srcDir(layout.buildDirectory.dir("$coreGeneratedRoot/jniLibs"))
        java.srcDir(if (aiEnabled) "src/ai/java" else "src/noAi/java")
        java.srcDir(if (meshEnabled) "src/mesh/java" else "src/noMesh/java")
        if (meshEnabled) java.srcDir("../../third_party/btleplug-0.12.0/java")
        assets.srcDir(layout.buildDirectory.dir("generated/notices"))
        assets.srcDir(layout.buildDirectory.dir("generated/webAssets"))
        if (aiEnabled) assets.srcDir(layout.buildDirectory.dir("generated/rerankInputs"))
    }
    buildTypes {
        release { isMinifyEnabled = false }
    }
}

dependencies {
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    // UniFFI's Android cleaner emits binary-retained API annotations.
    compileOnly("androidx.annotation:annotation:1.9.1")
}
dependencyLocking { lockAllConfigurations() }

val buildCore by tasks.registering(Exec::class) {
    group = "build"
    description = "Build the release ARM64 Rust core and generate matching UniFFI bindings."
    workingDir(rootProject.projectDir.parentFile)
    val arguments = mutableListOf("powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "android/build-core.ps1")
    if (aiEnabled) arguments += "-Ai"
    if (meshEnabled) arguments += "-Mesh"
    commandLine(arguments)
    onlyIf { !providers.gradleProperty("skipCoreBuild").isPresent }
}
val verifyCoreArtifacts by tasks.registering {
    dependsOn(buildCore)
    doLast {
        check(layout.buildDirectory.file("$coreGeneratedRoot/jniLibs/arm64-v8a/libtonalli_core.so").get().asFile.isFile) {
            "Missing ARM64 core; run buildCore before using -PskipCoreBuild"
        }
        check(layout.buildDirectory.file("$coreGeneratedRoot/jniLibs/arm64-v8a/libtonalli_maps.so").get().asFile.isFile) { "Missing ARM64 map extractor" }
        check(layout.buildDirectory.dir("$coreGeneratedRoot/uniffi").get().asFile.walkTopDown().any { it.extension == "kt" }) {
            "Missing generated UniFFI Kotlin bindings"
        }
    }
}
val packageNotices by tasks.registering(Sync::class) {
    from(listOf("LICENSE", "NOTICE", "LICENSES.md").map { rootProject.file("../$it") })
    from(rootProject.file("licenses"))
    from(rootProject.file("../native/maps-extract/licenses")) { into("country-extractor") }
    if (meshEnabled) {
        from(rootProject.file("../third_party/mesh-gateway-licenses")) { into("mesh-gateway") }
        from(rootProject.file("../crates/meshtastic-fork/TONALLI-CHANGES.md")) { into("mesh-gateway/meshtastic-0.1.9") }
    }
    into(layout.buildDirectory.dir("generated/notices/licenses"))
}
val packageRerankInputs by tasks.registering(Sync::class) {
    from(listOf("ai-rerank-inputs-v1.json", "ai-rerank-questions-v1.json").map { rootProject.file("../docs/spike/$it") })
    into(layout.buildDirectory.dir("generated/rerankInputs"))
}
val buildWeb by tasks.registering(Exec::class) {
    group = "build"
    description = "Build and checksum-package the shared React UI."
    workingDir(rootProject.projectDir.parentFile)
    commandLine("powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "android/build-web.ps1")
    onlyIf { !providers.gradleProperty("skipWebBuild").isPresent }
}
val verifyWebArtifacts by tasks.registering {
    dependsOn(buildWeb)
    doLast {
        val assets = layout.buildDirectory.dir("generated/webAssets").get().asFile
        val manifestFile = assets.resolve("ui-manifest.json")
        check(manifestFile.isFile && manifestFile.length() <= 512 * 1024) { "Missing or oversized packaged UI manifest" }
        val manifest = groovy.json.JsonSlurper().parse(manifestFile) as Map<*, *>
        check(manifest["version"] == 1 && (manifest["id"] as? String)?.matches(Regex("[a-f0-9]{64}")) == true)
        val records = manifest["files"] as List<*>
        check(records.size in 1..2000)
        val root = assets.resolve("ui")
        val paths = mutableSetOf<String>()
        var total = 0L
        for (record in records) {
            val entry = record as Map<*, *>
            val path = entry["path"] as String
            check(path.length in 1..512 && path.none { it <= '\u001f' || it == '\u007f' || it == '\\' || it == ':' } &&
                path.split('/').none { it in setOf("", ".", "..") } && paths.add(path))
            val length = entry["bytes"] as Number
            check(length.toString().matches(Regex("[0-9]+")) && length.toLong() <= 32L * 1024 * 1024)
            total += length.toLong()
            check(total <= 32L * 1024 * 1024)
            var file = root
            for (part in path.split('/')) {
                file = file.resolve(part)
                check(!Files.isSymbolicLink(file.toPath()))
            }
            check(file.isFile && file.length() == length.toLong()) { "UI length mismatch: $path" }
            val hash = MessageDigest.getInstance("SHA-256")
            file.inputStream().use { input ->
                val buffer = ByteArray(64 * 1024)
                while (true) { val count = input.read(buffer); if (count < 0) break; hash.update(buffer, 0, count) }
            }
            check(hash.digest().joinToString("") { "%02x".format(it) } == entry["sha256"]) { "UI checksum mismatch: $path" }
        }
        check("index.html" in paths)
        val actual = root.walkTopDown().onEnter { directory ->
            check(!Files.isSymbolicLink(directory.toPath())); true
        }.filter { it.isFile }.map { it.relativeTo(root).invariantSeparatorsPath }.toSet()
        check(actual == paths) { "Unlisted packaged UI files" }
    }
}
tasks.named("preBuild") {
    dependsOn(verifyCoreArtifacts, verifyWebArtifacts, packageNotices)
    if (aiEnabled) dependsOn(packageRerankInputs)
}

val hostReaderClasses = layout.buildDirectory.dir("host-reader-tests")
val compileReaderTests by tasks.registering(JavaExec::class) {
    maxHeapSize = "256m"
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostReaderClasses.get().asFile.absolutePath,
            file("src/main/java/org/tonalli/spike/saf/SafSelections.kt").absolutePath,
            file("src/main/java/org/tonalli/spike/reader/ReaderCatalogPolicy.kt").absolutePath,
            file("src/main/java/org/tonalli/spike/reader/ReaderFolderPolicy.kt").absolutePath,
            file("src/main/java/org/tonalli/spike/reader/ReaderIndexPolicy.kt").absolutePath,
            file("src/main/java/org/tonalli/spike/index/IndexTransferSpec.kt").absolutePath,
            file("src/hostTest/kotlin/org/tonalli/spike/reader/ReaderIndexTest.kt").absolutePath,
            file("src/hostTest/kotlin/org/tonalli/spike/reader/ReaderCatalogTest.kt").absolutePath)
    }
}
tasks.register<JavaExec>("checkReader") {
    group = "verification"
    maxHeapSize = "128m"
    dependsOn(compileReaderTests)
    classpath = files(hostReaderClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.reader.ReaderCatalogTestKt")
}

val hostIndexClasses = layout.buildDirectory.dir("host-index-tests")
val compileIndexSnapshotTests by tasks.registering(JavaExec::class) {
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostIndexClasses.get().asFile.absolutePath,
            file("src/main/java/org/tonalli/spike/index/IndexSnapshot.kt").absolutePath,
            file("src/hostTest/kotlin/org/tonalli/spike/index/IndexSnapshotTest.kt").absolutePath)
    }
}
tasks.register<JavaExec>("checkIndexSnapshot") {
    group = "verification"
    dependsOn(compileIndexSnapshotTests)
    classpath = files(hostIndexClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.index.IndexSnapshotTestKt")
}

val hostWebClasses = layout.buildDirectory.dir("host-web-tests")
val compileWebShellTests by tasks.registering(JavaExec::class) {
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostWebClasses.get().asFile.absolutePath)
        args(listOf("LocalOriginPolicy.kt", "WorkerSession.kt", "UiAssets.kt", "WebLaunchPolicy.kt").map {
            file("src/main/java/org/tonalli/spike/web/$it").absolutePath
        })
        args(file("src/hostTest/kotlin/org/tonalli/spike/web/WebShellTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/index/PreparationTicketTest.kt").absolutePath)
        args(file("src/main/java/org/tonalli/spike/index/PreparationTicketPolicy.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/web/MediaFolderTest.kt").absolutePath)
        args(file("src/main/java/org/tonalli/spike/media/MediaFolders.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/web/WebLaunchPolicyTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/web/UiAssetsConcurrencyTest.kt").absolutePath)
    }
}
tasks.register<JavaExec>("checkWebShell") {
    group = "verification"
    dependsOn(compileWebShellTests)
    classpath = files(hostWebClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.web.WebShellTestKt")
}
tasks.register<JavaExec>("checkUiAssetsConcurrency") {
    group = "verification"
    dependsOn(compileWebShellTests)
    maxHeapSize = "256m"
    classpath = files(hostWebClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.web.UiAssetsConcurrencyTestKt")
}

val hostNotesClasses = layout.buildDirectory.dir("host-notes-tests")
val compileNotesProbeTests by tasks.registering(JavaExec::class) {
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostNotesClasses.get().asFile.absolutePath,
            file("src/main/java/org/tonalli/spike/web/WorkerSession.kt").absolutePath,
            file("src/main/java/org/tonalli/spike/notes/VaultFiles.kt").absolutePath,
            file("src/main/java/org/tonalli/spike/notes/NotesActionGate.kt").absolutePath,
            file("src/hostTest/kotlin/org/tonalli/spike/notes/NotesProbeTest.kt").absolutePath)
    }
}
tasks.register<JavaExec>("checkNotesProbe") {
    group = "verification"
    dependsOn(compileNotesProbeTests)
    classpath = files(hostNotesClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.notes.NotesProbeTestKt")
}

val hostBenchmarkClasses = layout.buildDirectory.dir("host-benchmark-tests")
val compileBenchmarkTests by tasks.registering(JavaExec::class) {
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostBenchmarkClasses.get().asFile.absolutePath)
        args(listOf("BenchmarkSeries.kt", "BenchmarkRequest.kt", "BenchmarkFiles.kt").map {
            file("src/main/java/org/tonalli/spike/benchmark/$it").absolutePath
        })
        args(file("src/hostTest/kotlin/org/tonalli/spike/benchmark/BenchmarkTest.kt").absolutePath)
    }
}
tasks.register<JavaExec>("checkBenchmark") {
    group = "verification"
    dependsOn(compileBenchmarkTests)
    classpath = files(hostBenchmarkClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.benchmark.BenchmarkTestKt")
}

val hostSafClasses = layout.buildDirectory.dir("host-saf-tests")
val compileSafTests by tasks.registering(JavaExec::class) {
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostSafClasses.get().asFile.absolutePath)
        args(listOf("SafSelections.kt", "DescriptorTransfer.kt", "SafPublication.kt", "SafReadFence.kt", "SafBenchmarkContract.kt", "SafBenchmarkRefusal.kt", "SafErrorText.kt").map {
            file("src/main/java/org/tonalli/spike/saf/$it").absolutePath
        })
        args(file("src/hostTest/kotlin/org/tonalli/spike/saf/SafTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/saf/ProbeInputsTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/saf/SafBenchmarkTest.kt").absolutePath)
        args(file("src/main/java/org/tonalli/spike/probe/ProbeInputDirectories.kt").absolutePath)
        args(file("src/main/java/org/tonalli/spike/index/IndexSnapshot.kt").absolutePath)
    }
}

tasks.register<JavaExec>("checkSaf") {
    group = "verification"
    dependsOn(compileSafTests)
    classpath = files(hostSafClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.saf.SafTestKt")
}

val hostTransferClasses = layout.buildDirectory.dir("host-index-transfer-tests")
val compileIndexTransferTests by tasks.registering(JavaExec::class) {
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostTransferClasses.get().asFile.absolutePath)
        args(listOf("IndexTransferSpec.kt", "TrustedIndexInstaller.kt", "TrustedIndexInput.kt", "IndexQuerySeries.kt").map {
            file("src/main/java/org/tonalli/spike/index/$it").absolutePath
        })
        args(file("src/hostTest/kotlin/org/tonalli/spike/index/IndexTransferTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/index/TrustedIndexInputTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/index/IndexQuerySeriesTest.kt").absolutePath)
        args(file("src/main/java/org/tonalli/spike/benchmark/BenchmarkSeries.kt").absolutePath)
    }
}

tasks.register<JavaExec>("checkIndexTransfer") {
    group = "verification"
    dependsOn(compileIndexTransferTests)
    classpath = files(hostTransferClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.index.IndexTransferTestKt")
}

val hostLanClasses = layout.buildDirectory.dir("host-lan-tests")
val compileLanTests by tasks.registering(JavaExec::class) {
    maxHeapSize = "256m"
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostLanClasses.get().asFile.absolutePath)
        args(fileTree("src/main/java/org/tonalli/spike/lan") { include("LanLifecycle.kt", "LanBindPolicy.kt", "LanAddresses.kt", "LanHandoff.kt") }.files.map { it.absolutePath })
        args(file("src/hostTest/kotlin/org/tonalli/spike/lan/LanTest.kt").absolutePath)
    }
}
tasks.register<JavaExec>("checkLan") {
    group = "verification"
    maxHeapSize = "128m"
    dependsOn(compileLanTests)
    classpath = files(hostLanClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.lan.LanTestKt")
}

val hostAiModelClasses = layout.buildDirectory.dir("host-ai-model-tests")
val compileAiModelTests by tasks.registering(JavaExec::class) {
    maxHeapSize = "256m"
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostAiModelClasses.get().asFile.absolutePath)
        args(fileTree("src/main/java/org/tonalli/spike/ai") { include("AiModelSpec.kt", "AiModelFiles.kt") }.files.map { it.absolutePath })
        args(file("src/hostTest/kotlin/org/tonalli/spike/ai/AiModelFilesTest.kt").absolutePath)
    }
}

tasks.register<JavaExec>("checkAiModelFiles") {
    group = "verification"
    maxHeapSize = "128m"
    dependsOn(compileAiModelTests)
    classpath = files(hostAiModelClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.ai.AiModelFilesTestKt")
}

val hostAiRunClasses = layout.buildDirectory.dir("host-ai-run-tests")
val compileAiRunTests by tasks.registering(JavaExec::class) {
    maxHeapSize = "256m"
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", hostAiRunClasses.get().asFile.absolutePath)
        args(fileTree("src/main/java/org/tonalli/spike/ai") { include("AiRunGate.kt", "AiModelSpec.kt", "AiMeasurementRun.kt", "AiEmbeddingProbe.kt", "AiProbeRequest.kt", "AiFixtureEvidence.kt", "AiRerankInput.kt", "AiAnswerCase.kt") }.files.map { it.absolutePath })
        args(file("src/main/java/org/tonalli/spike/web/WorkerSession.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/ai/AiRunGateTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/ai/AiMeasurementRunTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/ai/AiEmbeddingProbeTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/ai/AiFixtureEvidenceTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/ai/AiRerankInputTest.kt").absolutePath)
        args(file("src/hostTest/kotlin/org/tonalli/spike/ai/AiAnswerCaseTest.kt").absolutePath)
    }
}

tasks.register<JavaExec>("checkAiRunGate") {
    group = "verification"
    maxHeapSize = "128m"
    dependsOn(compileAiRunTests)
    classpath = files(hostAiRunClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.spike.ai.AiRunGateTestKt")
}
