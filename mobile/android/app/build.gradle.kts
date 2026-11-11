plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.dragonglass.app"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.dragonglass.app"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }

    kotlin {
        compilerOptions {
            jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
        }
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(composeBom)

    implementation("androidx.core:core-ktx:1.19.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.11.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.11.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.documentfile:documentfile:1.1.0")

    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")

    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.11.0")

    // Git sync on mobile. GitHub uses HTTPS tokens; JGit keeps the repository format
    // identical to the desktop app's system git.
    implementation("org.eclipse.jgit:org.eclipse.jgit:7.8.0.202609011348-r")
    implementation("org.slf4j:slf4j-nop:2.0.17")

    // UniFFI-generated bindings load the Rust library through JNA.
    implementation("net.java.dev.jna:jna:5.17.0@aar")

    testImplementation("junit:junit:4.13.2")
    testImplementation("net.java.dev.jna:jna:5.17.0")

    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation("androidx.test:runner:1.7.0")

    debugImplementation("androidx.compose.ui:ui-tooling")
}

// JVM unit tests exercise the same UniFFI bridge against the host build of the engine.
tasks.withType<Test>().configureEach {
    systemProperty("jna.library.path", rootProject.file("../../target/debug").absolutePath)
}

// The Kotlin side compiles without the Rust library, but the app cannot open a vault
// without it. Warn early instead of crashing on the device with UnsatisfiedLinkError.
tasks.register("checkNativeLibraries") {
    doFirst {
        val jni = file("src/main/jniLibs")
        val found = jni.exists() && jni.walkTopDown().any { it.name == "libdragonglass_ffi.so" }
        if (!found) {
            logger.warn(
                "No libdragonglass_ffi.so found in app/src/main/jniLibs. " +
                    "Run mobile/android/scripts/build-native.ps1 before installing the app."
            )
        }
    }
}

tasks.named("preBuild").configure { dependsOn("checkNativeLibraries") }

