class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.17.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.17.0/scriba-x86_64-apple-darwin"
    sha256 "e96c9ce147e12d7c6ed98b11e2220aaa95e98c449c1806191a51bf9e7d7aada6"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.17.0/scriba-aarch64-apple-darwin"
    sha256 "2907c00e181b6e2c0ae046852b752029a306aa1483e557dc923c0241e6add776"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
