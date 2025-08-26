class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.7.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.7.1/scriba-x86_64-apple-darwin"
    sha256 "012217556a827ec49751bbd168a4b2cb55afc53e6308488992c4264fbf6cd4be"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.7.1/scriba-aarch64-apple-darwin"
    sha256 "5cd942a36a94523a5867c53e22677809c2f24ca9561470ff4fdeb415fea3e86c"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
