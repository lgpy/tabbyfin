import com.android.build.api.variant.FilterConfiguration

plugins {
    id("com.android.application") version "9.5.0"
}

// The app's version is the workspace's (Cargo.toml), so a release tag only bumps it there.
val version: String = Regex("""(?m)^version = "(\d+\.\d+\.\d+)"""")
    .find(rootDir.resolve("../../Cargo.toml").readText())
    ?.groupValues?.get(1)
    ?: error("no version in Cargo.toml")
// 0.2.0 is 200, 1.12.3 is 11203; times ten, so each ABI's APK has its own last digit.
val versionBase = version.split(".").map(String::toInt).let { (major, minor, patch) ->
    major * 10_000 + minor * 100 + patch
}
val abiDigits = mapOf("arm64-v8a" to 1, "x86_64" to 2)

// The release signing key, from the environment (`build.sh --sign`, CI's secrets): never in
// the repository.
val keystore = System.getenv("TABBYFIN_KEYSTORE")

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
        // A debug APK holds every ABI built, and ends in 0.
        versionCode = versionBase * 10
        versionName = version
    }

    signingConfigs {
        if (keystore != null) {
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("TABBYFIN_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("TABBYFIN_KEY_ALIAS")
                keyPassword = System.getenv("TABBYFIN_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("release")
            // R8 shrinks Media3 and AndroidX to what's used; proguard-rules.pro keeps what
            // the Rust side calls by name.
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    // A release APK per ABI: a phone needs only its own, and each is half the size of both.
    splits {
        abi {
            isEnable = gradle.startParameter.taskNames.any { it.contains("Release") }
            reset()
            // The ABIs build.sh built, so a one-ABI build doesn't make an empty APK for another.
            include(*abiDigits.keys.filter { file("src/main/jniLibs/$it").isDirectory }.toTypedArray())
            isUniversalApk = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        // Keep the library's symbols, for stack traces in logcat (`build.sh --sign` strips them in
        // the Rust build).
        jniLibs.keepDebugSymbols += "**/libtabbyfin.so"
    }
}

androidComponents {
    onVariants(selector().withBuildType("release")) { variant ->
        variant.outputs.forEach { output ->
            val abi = output.filters.find { it.filterType == FilterConfiguration.FilterType.ABI }?.identifier
            abi?.let { output.versionCode.set(versionBase * 10 + abiDigits.getValue(it)) }
        }
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
