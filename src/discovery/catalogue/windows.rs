use super::KnownLocation;
use crate::model::Category::{
    AiToolCache, AndroidSdk, Docker, EditorCache, GodotCache, PackageManagerCache, UnityLibrary,
};
use crate::model::LocationKind::{Artifact, ReportOnly, SharedCache, ToolState};

pub(super) static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "npm cache" => "AppData/Local/npm-cache/_cacache", PackageManagerCache, SharedCache, &[];
    "pnpm store" => "AppData/Local/pnpm/store", PackageManagerCache, SharedCache, &[];
    "Yarn cache" => "AppData/Local/Yarn/Cache", PackageManagerCache, SharedCache, &[];
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache, &[];
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache, &["bun"];
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache, &["cargo"];
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache, &[];
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState, &[];
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache, &[];
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState, &["go"];
    "Go build cache" => "AppData/Local/go-build", PackageManagerCache, SharedCache, &["go"];
    "pip cache" => "AppData/Local/pip/Cache", PackageManagerCache, SharedCache, &[];
    "node-gyp headers" => "AppData/Local/node-gyp/Cache", PackageManagerCache, SharedCache, &[];
    "Deno cache" => "AppData/Local/deno", PackageManagerCache, SharedCache, &["deno"];
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache, &[];
    "Playwright browsers" => "AppData/Local/ms-playwright", PackageManagerCache, SharedCache, &[];
    "uv cache" => "AppData/Local/uv/cache", PackageManagerCache, SharedCache, &["uv", "uvx"];

    // Godot's own cache directory is the one path its documentation gives two
    // answers for, so it is not catalogued here: `%TEMP%\\Godot` in the data
    // paths guide, `%LOCALAPPDATA%\\Godot` in the class reference. Export
    // templates are stated once and agree with the other platforms.
    "Unity package cache" => "AppData/Local/Unity/cache/upm", UnityLibrary, SharedCache, &["Unity"];
    "Unity Asset Store packages" => "AppData/Roaming/Unity/Asset Store-5.x", UnityLibrary, ToolState, &["Unity"];
    "Unity build cache" => "AppData/Local/Unity/Caches/bee", UnityLibrary, ToolState, &["Unity"];
    "Godot export templates" => "AppData/Roaming/Godot/export_templates", GodotCache, SharedCache, &["Godot"];

    // The emulator directory is left out here: Android's own documentation
    // prints two different Windows layouts for it, and neither agrees with the
    // SDK root it documents elsewhere.
    "Android system images" => "AppData/Local/Android/Sdk/system-images", AndroidSdk, SharedCache, &["emulator", "qemu-system-x86_64", "studio"];

    "Docker disk image" => "AppData/Local/Docker/wsl/data", Docker, ReportOnly, &["com.docker.backend", "Docker Desktop", "docker"];

    "Ollama models" => ".ollama/models", AiToolCache, ToolState, &["ollama"];

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState, &["claude"];
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState, &["claude"];
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState, &["claude"];
    "Codex logs" => ".codex/log", AiToolCache, ToolState, &["codex"];
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState, &["opencode"];

    "JetBrains caches" => "AppData/Local/JetBrains", EditorCache, ToolState, &["idea", "pycharm", "webstorm", "goland", "clion", "phpstorm", "rubymine", "rustrover", "datagrip"];
    "VS Code cache" => "AppData/Roaming/Code/Cache", EditorCache, Artifact, &["Code"];
    "VS Code cached data" => "AppData/Roaming/Code/CachedData", EditorCache, Artifact, &["Code"];
    "VS Code GPU cache" => "AppData/Roaming/Code/GPUCache", EditorCache, Artifact, &["Code"];
};
