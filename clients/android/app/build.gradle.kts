import java.io.File
import java.util.Properties
import groovy.json.JsonSlurper
import groovy.json.JsonOutput

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

// google-services.json 不入库:有文件才启用推送插件。开源构建照常可用,只是没有推送。
if (file("google-services.json").exists()) {
    apply(plugin = libs.plugins.google.services.get().pluginId)
}

// Rust cloud-ffi:arm64 的 .so 与 UniFFI Kotlin 绑定都生成到 build/,不入库。
val repoRoot: File = rootProject.projectDir.parentFile.parentFile
// Consume only Mobile Push app credentials; keep the downloaded EMAS file outside version control.
val aliyunServicesFile = file("aliyun-emas-services.json")
val aliyunConfig = if (aliyunServicesFile.isFile) {
    val document = JsonSlurper().parse(aliyunServicesFile) as Map<*, *>
    val config = document["config"] as Map<*, *>
    require(config["emas.packageName"] == "ai.eclosion.octoterm.android") { "EMAS package name does not match Android applicationId" }
    config
} else emptyMap<Any, Any>()
// 签名文件放在仓库同级的 OctotermSecrets，其它机器保持这个相对位置即可（兼顾 .worktrees 子目录）。
val signingPropertiesFile = listOfNotNull(
    File(repoRoot.parentFile, "OctotermSecrets/octoterm-release.signing.properties"),
    repoRoot.parentFile?.parentFile?.parentFile?.let { File(it, "OctotermSecrets/octoterm-release.signing.properties") },
).firstOrNull { it.isFile } ?: File(repoRoot.parentFile, "OctotermSecrets/octoterm-release.signing.properties")
val signingProperties = Properties()
val hasOctotermSigning = signingPropertiesFile.isFile
if (hasOctotermSigning) {
    signingPropertiesFile.inputStream().use { signingProperties.load(it) }
}

val ffiJniDir = layout.buildDirectory.dir("generated/ffi/jniLibs")
val ffiKotlinDir = layout.buildDirectory.dir("generated/ffi/kotlin")
// 构建前提:PATH 上要有 cargo 与 cargo-ndk,并设置 ANDROID_NDK_HOME 指向 NDK。
// 只编 arm64-v8a,APK 因此只支持 arm64 设备(x86_64 模拟器装不上)。
val buildCloudFfi by tasks.registering(Exec::class) {
    description = "构建 cloud-ffi(arm64-v8a)并生成 Kotlin 绑定"
    workingDir = repoRoot
    // Windows 上的 System32\bash.exe 是 WSL，看不到 Windows 环境变量，也配不了本机 cargo。
    val gitBash = File("C:/Program Files/Git/bin/bash.exe")
    val bashExe = if (gitBash.isFile) gitBash.absolutePath else "bash"
    commandLine(
        bashExe, "scripts/build-android-ffi.sh",
        ffiJniDir.get().asFile.absolutePath,
        ffiKotlinDir.get().asFile.absolutePath,
    )
    inputs.dir(File(repoRoot, "crates/client-core/src"))
    inputs.dir(File(repoRoot, "crates/cloud-ffi/src"))
    inputs.dir(File(repoRoot, "crates/cloud-client/src"))
    inputs.file(File(repoRoot, "scripts/build-android-ffi.sh"))
    // 依赖版本/特性变化也要重编
    inputs.file(File(repoRoot, "Cargo.toml"))
    inputs.file(File(repoRoot, "Cargo.lock"))
    outputs.dir(ffiJniDir)
    outputs.dir(ffiKotlinDir)
}
tasks.named("preBuild") { dependsOn(buildCloudFfi) }

android {
    namespace = "ai.eclosion.octoterm.android"
    compileSdk = 36

    defaultConfig {
        buildConfigField("String", "ALIYUN_APP_KEY", JsonOutput.toJson(aliyunConfig["emas.appKey"]?.toString() ?: ""))
        buildConfigField("String", "ALIYUN_APP_SECRET", JsonOutput.toJson(aliyunConfig["emas.appSecret"]?.toString() ?: ""))
        applicationId = "ai.eclosion.octoterm.android"
        minSdk = 26
        targetSdk = 36
        versionCode = 4
        versionName = "0.2.3"
        ndk { abiFilters += "arm64-v8a" }
    }

    sourceSets["main"].jniLibs.srcDir(ffiJniDir)
    sourceSets["main"].java.srcDir(ffiKotlinDir)

    signingConfigs {
        if (hasOctotermSigning) {
            create("octoterm") {
                val configuredStore = File(signingProperties.getProperty("storeFile"))
                storeFile = if (configuredStore.isAbsolute) {
                    configuredStore
                } else {
                    File(signingPropertiesFile.parentFile, configuredStore.path)
                }
                storePassword = signingProperties.getProperty("storePassword")
                keyAlias = signingProperties.getProperty("keyAlias")
                keyPassword = signingProperties.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        debug {
            if (hasOctotermSigning) {
                signingConfig = signingConfigs.getByName("octoterm")
            }
        }
        release {
            if (hasOctotermSigning) {
                signingConfig = signingConfigs.getByName("octoterm")
            }
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    testOptions {
        unitTests.all {
            val jnaPaths = listOfNotNull(
                System.getenv("CARGO_TARGET_DIR")?.let { File(it, "debug").absolutePath },
                File(repoRoot, "target/debug").absolutePath,
                repoRoot.parentFile?.parentFile?.let { File(it, "target/debug").absolutePath },
            ).distinct().joinToString(File.pathSeparator)
            it.systemProperty("jna.library.path", jnaPaths)
        }
    }
}

dependencies {
    implementation("com.aliyun.ams:alicloud-android-push:4.0.0")
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.activity.compose)
    implementation(libs.okhttp)
    implementation(libs.androidx.appcompat)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    debugImplementation(libs.androidx.compose.ui.tooling)
    testImplementation(libs.junit)
    testImplementation(libs.org.json)
    testImplementation(libs.jna)
    implementation(libs.jna) { artifact { type = "aar" } }
    implementation(platform(libs.firebase.bom))
    implementation(libs.firebase.messaging)
    implementation(libs.kotlinx.coroutines.android)
}
