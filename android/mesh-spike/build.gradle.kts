plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}
android {
    namespace = "org.tonalli.meshspike"
    compileSdk = 35
    defaultConfig {
        applicationId = "org.tonalli.meshspike"
        minSdk = 34
        targetSdk = 34
        versionCode = 1
        versionName = "0.1-spike"
        ndk { abiFilters += "arm64-v8a" }
    }
    sourceSets.getByName("main") {
        java.srcDir("../../third_party/btleplug-0.12.0/java")
        jniLibs.srcDir("build/generated/jniLibs")
        assets.srcDir("../../third_party/mesh-gateway-licenses")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    packaging { jniLibs.useLegacyPackaging = false }
}
