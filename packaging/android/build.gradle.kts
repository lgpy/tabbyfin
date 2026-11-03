plugins {
    id("com.android.application") version "9.5.0"
}

android {
    namespace = "io.github.lgpy.tabbyfin"
    compileSdk = 36
    // The NDK the Rust library is built with (`build.sh`); Gradle only packages its output.
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "io.github.lgpy.tabbyfin"
        // AHardwareBuffer and Vulkan 1.0 (docs/android.md).
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.2.0"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        // Keep the library's symbols in debug builds, for stack traces in logcat.
        jniLibs.keepDebugSymbols += "**/libtabbyfin.so"
    }
}

dependencies {
    // The version android-activity's GameActivity glue is written against (its README).
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    // Media3, the default player (Media3Bridge.java), with HLS for the server's transcodes.
    implementation("androidx.media3:media3-exoplayer:1.8.0")
    implementation("androidx.media3:media3-exoplayer-hls:1.8.0")
    // Jellyfin's ffmpeg audio decoders for Media3 (DTS, TrueHD…), as Findroid uses; the version
    // matches Media3's.
    implementation("org.jellyfin.media3:media3-ffmpeg-decoder:1.8.0+1")
}
