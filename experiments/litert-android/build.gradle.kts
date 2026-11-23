import java.security.MessageDigest
import java.util.zip.ZipFile

plugins {
    id("com.android.application") version "8.7.2"
    id("org.jetbrains.kotlin.android") version "2.4.0"
}

val branding = groovy.json.JsonSlurper().parse(file("../../branding.json")) as Map<*, *>
android {
    namespace = "org.tonalli.litert"
    compileSdk = 34
    defaultConfig {
        applicationId = "org.tonalli.spike.litertprobe"
        minSdk = 34
        targetSdk = 34
        versionCode = 1
        versionName = "0.1-litert-experiment"
        resValue("string", "app_name", "${branding["displayName"]} LiteRT experiment")
        ndk { abiFilters += "arm64-v8a" }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("generated/notices"))
}
kotlin { compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) } }
val sdkNotices by configurations.creating {
    isCanBeResolved = true
    isCanBeConsumed = false
    isTransitive = false
}
dependencies {
    implementation("com.google.ai.edge.litertlm:litertlm-android:0.19.1")
    add(sdkNotices.name, "com.google.ai.edge.litertlm:litertlm-android:0.19.1@aar")
}
dependencyLocking { lockAllConfigurations() }

val packageNotices by tasks.registering {
    inputs.files(sdkNotices)
    outputs.dir(layout.buildDirectory.dir("generated/notices"))
    doLast {
        val aar = sdkNotices.singleFile
        check(MessageDigest.getInstance("SHA-256").digest(aar.readBytes()).joinToString("") { "%02x".format(it) } ==
            "341f66349ac962e0b6733b8da419b045a4834fca2f7db0bd13e049ce1cfcca72") { "SDK notice source checksum mismatch" }
        val out = layout.buildDirectory.dir("generated/notices/licenses").get().asFile.apply { mkdirs() }
        ZipFile(aar).use { zip ->
            for (name in listOf("LICENSE", "THIRD_PARTY_NOTICE.txt")) {
                zip.getInputStream(zip.getEntry(name)).use { input -> out.resolve("LiteRT-$name").outputStream().use { output -> input.copyTo(output) } }
            }
        }
        file("../../LICENSE").copyTo(out.resolve("Tonalli-LICENSE"), overwrite = true)
    }
}
tasks.named("preBuild") { dependsOn(packageNotices) }

val contractClasses = layout.buildDirectory.dir("contract-tests")
val compileContract by tasks.registering(JavaExec::class) {
    maxHeapSize = "256m"
    classpath = configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.jetbrains.kotlin.cli.jvm.K2JVMCompiler")
    doFirst {
        args("-jvm-target", "17", "-no-stdlib", "-no-reflect", "-classpath", classpath.asPath,
            "-d", contractClasses.get().asFile.absolutePath,
            file("src/main/kotlin/org/tonalli/litert/ProbeContract.kt").absolutePath,
            file("src/test/kotlin/org/tonalli/litert/ProbeContractTest.kt").absolutePath)
    }
}
tasks.register<JavaExec>("checkContract") {
    dependsOn(compileContract)
    maxHeapSize = "128m"
    classpath = files(contractClasses) + configurations.getByName("kotlinCompilerClasspath")
    mainClass.set("org.tonalli.litert.ProbeContractTestKt")
}
