use super::KnownLocation;
use crate::model::Category::{
    AiToolCache, AndroidSdk, Docker, EditorCache, GodotCache, PackageManagerCache, UnityLibrary,
    XcodeDerivedData,
};
use crate::model::LocationKind::{Artifact, ReportOnly, SharedCache, ToolState};

// `Archives` and `CoreSimulator/Devices` are catalogued as `ReportOnly`. They
// are usually the two heaviest directories Xcode owns, and neither is ours to
// remove: an archive holds the dSYMs that symbolicate crashes from a release
// already in users' hands, and a simulator device holds whatever was installed
// and configured inside it. Both name the operation that reclaims them safely.
// `CoreSimulator/Caches` next to them is an ordinary artifact.
pub(super) static PLATFORM_LOCATIONS: &[KnownLocation] = locations! {
    "Xcode DerivedData" => "Library/Developer/Xcode/DerivedData", XcodeDerivedData, Artifact, &["Xcode"];
    "Simulator devices" => "Library/Developer/CoreSimulator/Devices", XcodeDerivedData, ReportOnly, &["Xcode", "Simulator"];
    "Xcode archives" => "Library/Developer/Xcode/Archives", XcodeDerivedData, ReportOnly, &["Xcode"];
    "Xcode iOS device support" => "Library/Developer/Xcode/iOS DeviceSupport", XcodeDerivedData, Artifact, &["Xcode"];
    "Xcode watchOS device support" => "Library/Developer/Xcode/watchOS DeviceSupport", XcodeDerivedData, Artifact, &["Xcode"];
    "Xcode tvOS device support" => "Library/Developer/Xcode/tvOS DeviceSupport", XcodeDerivedData, Artifact, &["Xcode"];
    "Simulator caches" => "Library/Developer/CoreSimulator/Caches", XcodeDerivedData, Artifact, &["Xcode", "Simulator"];
    "Xcode cache" => "Library/Caches/com.apple.dt.Xcode", XcodeDerivedData, Artifact, &["Xcode"];

    "npm cache" => ".npm/_cacache", PackageManagerCache, SharedCache, &[];
    "pnpm store" => "Library/pnpm/store", PackageManagerCache, SharedCache, &[];
    "pnpm cache" => "Library/Caches/pnpm", PackageManagerCache, SharedCache, &[];
    "Yarn cache" => "Library/Caches/Yarn", PackageManagerCache, SharedCache, &[];
    "Yarn Berry cache" => ".yarn/berry/cache", PackageManagerCache, SharedCache, &[];
    "Bun cache" => ".bun/install/cache", PackageManagerCache, SharedCache, &["bun"];
    "Cargo registry" => ".cargo/registry", PackageManagerCache, SharedCache, &["cargo"];
    "Gradle cache" => ".gradle/caches", PackageManagerCache, SharedCache, &[];
    "Maven repository" => ".m2/repository", PackageManagerCache, ToolState, &[];
    "NuGet packages" => ".nuget/packages", PackageManagerCache, SharedCache, &[];
    "Go module cache" => "go/pkg/mod", PackageManagerCache, ToolState, &["go"];
    "Go build cache" => "Library/Caches/go-build", PackageManagerCache, SharedCache, &["go"];
    "pip cache" => "Library/Caches/pip", PackageManagerCache, SharedCache, &[];
    "CocoaPods cache" => "Library/Caches/CocoaPods", PackageManagerCache, SharedCache, &[];
    "Homebrew downloads" => "Library/Caches/Homebrew", PackageManagerCache, SharedCache, &[];
    "node-gyp headers" => "Library/Caches/node-gyp", PackageManagerCache, SharedCache, &[];
    "TypeScript cache" => "Library/Caches/typescript", PackageManagerCache, SharedCache, &[];
    "Deno cache" => "Library/Caches/deno", PackageManagerCache, SharedCache, &["deno"];
    "Puppeteer browsers" => ".cache/puppeteer", PackageManagerCache, SharedCache, &[];
    "Playwright browsers" => "Library/Caches/ms-playwright", PackageManagerCache, SharedCache, &[];
    "uv cache" => ".cache/uv", PackageManagerCache, SharedCache, &["uv", "uvx"];

    "Unity package cache" => "Library/Caches/Unity/upm", UnityLibrary, SharedCache, &["Unity"];
    "Unity Asset Store packages" => "Library/Unity/Asset Store-5.x", UnityLibrary, ToolState, &["Unity"];
    "Unity build cache" => "Library/Unity/cache/bee", UnityLibrary, ToolState, &["Unity"];
    "Godot editor cache" => "Library/Caches/Godot", GodotCache, Artifact, &["Godot"];
    "Godot export templates" => "Library/Application Support/Godot/export_templates", GodotCache, SharedCache, &["Godot"];

    // Android's SDK downloads one copy of a system image for every project on
    // the machine; an emulator built from one holds whatever was installed
    // inside it, which is why only the first is cleanable.
    "Android system images" => "Library/Android/sdk/system-images", AndroidSdk, SharedCache, &["emulator", "qemu-system-aarch64", "studio"];
    "Android emulators" => ".android/avd", AndroidSdk, ReportOnly, &["emulator", "qemu-system-aarch64"];

    // One file holds every image, volume and container, including images built
    // here and pushed nowhere. Docker's own prune is the only safe way in.
    "Docker disk image" => "Library/Containers/com.docker.docker/Data/vms/0/data", Docker, ReportOnly, &["com.docker.backend", "Docker Desktop", "docker"];

    "Ollama models" => ".ollama/models", AiToolCache, ToolState, &["ollama"];

    "nvm download cache" => ".nvm/.cache", PackageManagerCache, SharedCache, &[];
    "Node versions (nvm)" => ".nvm/versions/node", PackageManagerCache, ToolState, &["node"];
    "pyenv build cache" => ".pyenv/cache", PackageManagerCache, SharedCache, &[];
    "Python versions (pyenv)" => ".pyenv/versions", PackageManagerCache, ToolState, &["python", "python3"];

    "Claude Code shell snapshots" => ".claude/shell-snapshots", AiToolCache, ToolState, &["claude"];
    "Claude Code IDE locks" => ".claude/ide", AiToolCache, ToolState, &["claude"];
    "Claude Code session environment" => ".claude/session-env", AiToolCache, ToolState, &["claude"];
    "Claude app cache" => "Library/Application Support/Claude/Cache", AiToolCache, ToolState, &["Claude"];
    "Claude app code cache" => "Library/Application Support/Claude/Code Cache", AiToolCache, ToolState, &["Claude"];
    "Codex logs" => ".codex/log", AiToolCache, ToolState, &["codex"];
    "opencode cache" => ".cache/opencode", AiToolCache, ToolState, &["opencode"];
    "opencode logs" => ".local/share/opencode/log", AiToolCache, ToolState, &["opencode"];
    "opencode tool output" => ".local/share/opencode/tool-output", AiToolCache, ToolState, &["opencode"];

    "JetBrains caches" => "Library/Caches/JetBrains", EditorCache, ToolState, &["idea", "pycharm", "webstorm", "goland", "clion", "phpstorm", "rubymine", "rustrover", "datagrip"];
    "VS Code cache" => "Library/Application Support/Code/Cache", EditorCache, Artifact, &["Code"];
    "VS Code cached data" => "Library/Application Support/Code/CachedData", EditorCache, Artifact, &["Code"];
    "VS Code GPU cache" => "Library/Application Support/Code/GPUCache", EditorCache, Artifact, &["Code"];
    "VS Code extension packages" => "Library/Application Support/Code/CachedExtensionVSIXs", EditorCache, Artifact, &["Code"];
};
