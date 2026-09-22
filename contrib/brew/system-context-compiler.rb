# trace:v1 id=ops.scc-brew-formula work=WORK-SCC-DISTRIBUTION title="Homebrew formula: prebuilt binaries installed as scc"
# Homebrew formula for System Context Compiler (scc).
#
# Copy into your own tap: <tap-repo>/Formula/system-context-compiler.rb
#   brew install carterlasalle/tap/system-context-compiler
#
# After each new release, regenerate with contrib/brew/bump.sh.
#
# The formula is deliberately NOT called `scc`: that name belongs to
# boyter/scc (a Go line counter) in homebrew-core, so `brew install scc`
# installs a different tool. The binary this formula installs IS `scc`.
#
# Installs the prebuilt release binary for Linux x86_64 and macOS arm64.
# Other platforms: `cargo install scc-cli` (see docs/INSTALL.md).
#
# There is no explicit `version` stanza: Homebrew scans it from the asset URL
# (`brew audit --strict` flags the duplication).

class SystemContextCompiler < Formula
  desc "Compile repositories into evidence-backed system context for coding agents"
  homepage "https://github.com/carterlasalle/scc"
  license "MIT"

  livecheck do
    url :stable
    strategy :github_latest
  end

  on_macos do
    on_arm do
      url "https://github.com/carterlasalle/scc/releases/download/v0.2.6/scc-0.2.6-Darwin-arm64"
      sha256 "919daf7bee47f6bed3ec58fae1cf23553ce46d26feec0ff1b5650c0de43f9ae0"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/carterlasalle/scc/releases/download/v0.2.6/scc-0.2.6-Linux-x86_64"
      sha256 "a9b41f94c0b759587c3b62c6f68ad27f21431c865deb23f6c1a18dae859cd640"
    end
  end

  def install
    # The download is named scc-<version>-<platform>; install it as `scc`.
    binary = Dir["scc-*"].first
    odie "no scc binary in the download" if binary.nil?

    bin.install binary => "scc"
  end

  def caveats
    <<~EOS
      Install SCC into a repository with:
        cd /path/to/your/repo && scc init && scc index
      Then wire your agent:
        scc setup claude      # or codex / opencode / hermes / omp / pi
    EOS
  end

  test do
    assert_match "scc", shell_output("#{bin}/scc --version")
  end
end
