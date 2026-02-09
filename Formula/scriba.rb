class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.15.1"

  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.15.1/scriba-x86_64-apple-darwin"
    sha256 "PLACEHOLDER_X86_64"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.15.1/scriba-aarch64-apple-darwin"
    sha256 "PLACEHOLDER_AARCH64"
  end

  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
