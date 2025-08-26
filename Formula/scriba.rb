class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.8.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.8.0/scriba-x86_64-apple-darwin"
    sha256 "27d833cb7b192ada80aff106a1be31b79e30ae5e131bfd4ed6829e38078b55d6"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.8.0/scriba-aarch64-apple-darwin"
    sha256 "d0d0da1013018168fc9381675959fd667a947af1d15a40e970755784196a0c46"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
