// SPDX-License-Identifier: AGPL-3.0-or-later
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.clapandpay.klimper.xchonnect"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    // Firebase Cloud Messaging carries the wake-up. The app needs a google-services.json
    // (docs/push.md); without one the token is never issued and Klimper polls instead.
    implementation("com.google.firebase:firebase-messaging:24.1.0")
    implementation("androidx.core:core-ktx:1.9.0")
    implementation(project(":tauri-android"))
}
