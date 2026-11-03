// The Android app's packaging: the Rust app (`crates/app`) as libtabbyfin.so, started by
// GameActivity. `bash packaging/android/build.sh` builds the library into `src/main/jniLibs/`
// and then runs Gradle; docs/android.md has the rest.
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "tabbyfin"
