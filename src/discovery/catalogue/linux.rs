use super::KnownLocation;
use crate::model::Category::{
    AiToolCache, AndroidSdk, Docker, EditorCache, GodotCache, PackageManagerCache, UnityLibrary,
};
use crate::model::LocationKind::{Artifact, ReportOnly, SharedCache, ToolState};

pub(super) static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "npm cache" => ".npm/_cacache", PackageManagerCache, SharedCache, &[];
    "pnpm store" => ".local/share/pnpm/store", PackageManagerCache, SharedCache, &[];
    "pnpm cache" => ".cache/pnpm", PackageManagerCache, SharedCache, &[];
    "Yarn cache" => ".cache/yarn", PackageManagerCache, SharedCache, &[];
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache, &[];
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache, &["bun"];
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache, &["cargo"];
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache, &[];
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState, &[];
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache, &[];
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState, &["go"];
    "Go build cache" => ".cache/go-build", PackageManagerCache, SharedCache, &["go"];
    "pip cache" => ".cache/pip", PackageManagerCache, SharedCache, &[];
    "node-gyp headers" => ".cache/node-gyp", PackageManagerCache, SharedCache, &[];
    "Deno cache" => ".cache/deno", PackageManagerCache, SharedCache, &["deno"];
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache, &[];
    "Playwright browsers" => ".cache/ms-playwright", PackageManagerCache, SharedCache, &[];
    "uv cache" => ".cache/uv", PackageManagerCache, SharedCache, &["uv", "uvx"];

    "Unity package cache" => ".cache/Unity/upm", UnityLibrary, SharedCache, &["Unity"];
    "Unity Asset Store packages" => ".local/share/unity3d/Asset Store-5.x", UnityLibrary, ToolState, &["Unity"];
    "Unity build cache" => ".cache/unity3d/bee", UnityLibrary, ToolState, &["Unity"];
    "Godot editor cache" => ".cache/godot", GodotCache, Artifact, &["Godot"];
    "Godot export templates" => ".local/share/godot/export_templates", GodotCache, SharedCache, &["Godot"];

    "Android system images" => "Android/Sdk/system-images", AndroidSdk, SharedCache, &["emulator", "qemu-system-x86_64", "studio"];
    "Android emulators" => ".android/avd", AndroidSdk, ReportOnly, &["emulator", "qemu-system-x86_64"];

    "Docker disk image" => ".docker/desktop/vms/0/data", Docker, ReportOnly, &["com.docker.backend", "docker"];

    "nvm download cache" => ".nvm/.cache", PackageManagerCache, SharedCache, &[];
    "Node versions (nvm)" => ".nvm/versions/node", PackageManagerCache, ToolState, &["node"];
    "pyenv build cache" => ".pyenv/cache", PackageManagerCache, SharedCache, &[];
    "Python versions (pyenv)" => ".pyenv/versions", PackageManagerCache, ToolState, &["python", "python3"];

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState, &["claude"];
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState, &["claude"];
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState, &["claude"];
    "Codex logs" => ".codex/log", AiToolCache, ToolState, &["codex"];
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState, &["opencode"];
    "opencode logs" => ".local/share/opencode/log", AiToolCache, ToolState, &["opencode"];
    "opencode tool output" => ".local/share/opencode/tool-output", AiToolCache, ToolState, &["opencode"];

    "JetBrains caches" => ".cache/JetBrains", EditorCache, ToolState, &["idea", "pycharm", "webstorm", "goland", "clion", "phpstorm", "rubymine", "rustrover", "datagrip"];
    "VS Code cache" => ".config/Code/Cache", EditorCache, Artifact, &["Code"];
    "VS Code cached data" => ".config/Code/CachedData", EditorCache, Artifact, &["Code"];
    "VS Code GPU cache" => ".config/Code/GPUCache", EditorCache, Artifact, &["Code"];
};
