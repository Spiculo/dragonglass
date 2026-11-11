# Keep the UniFFI/JNA bridge; release builds currently do not minify, but this makes the
# rules explicit for when they do.
-keep class uniffi.** { *; }
-keep class com.sun.jna.** { *; }
