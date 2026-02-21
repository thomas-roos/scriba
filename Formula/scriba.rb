class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.19.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.1/scriba-x86_64-apple-darwin"
    sha256 "719d3fa72f864699a8e5ad572a41968b3914f8e7b5565def38d814ab07ea00bb"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.1/scriba-aarch64-apple-darwin"
    sha256 "0400c79304649b79cac2003662d0cfc6f9e7e8071ec468ca7089051d1d1d33ac"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
