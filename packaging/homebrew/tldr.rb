# typed: false
# frozen_string_literal: true

# Homebrew formula for tldr. Lives in the johannes117/homebrew-tldr tap.
# Update `version`, `url`s, and `sha256`s on each release (the release
# workflow publishes `<asset>.sha256` files alongside each tarball).
class Tldr < Formula
  desc "Local-first PR review tool"
  homepage "https://github.com/johannes117/tldr"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/johannes117/tldr/releases/download/v#{version}/tldr-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_DARWIN_SHA256"
    end
    on_intel do
      url "https://github.com/johannes117/tldr/releases/download/v#{version}/tldr-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_DARWIN_SHA256"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/johannes117/tldr/releases/download/v#{version}/tldr-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_LINUX_SHA256"
    end
    on_intel do
      url "https://github.com/johannes117/tldr/releases/download/v#{version}/tldr-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_X86_64_LINUX_SHA256"
    end
  end

  def install
    bin.install "tldr"
  end

  test do
    assert_match "tldr", shell_output("#{bin}/tldr --version")
  end
end
