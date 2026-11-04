# R8 rules for the release APK. The Rust side finds the app's classes, their methods and
# their native methods by name (MainActivity's JNI exports, `load_class` and RegisterNatives
# for VideoSurface and Media3Bridge), so none of them may be renamed or removed.
-keep class io.github.lgpy.tabbyfin.** { *; }

# File and line numbers, so stack traces in logcat stay readable.
-keepattributes SourceFile,LineNumberTable
