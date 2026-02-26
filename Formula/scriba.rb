class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.19.3"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.3/scriba-x86_64-apple-darwin"
    sha256 "4ea884aedc2f2a63952a2de47b0b2579d1764363688e182c5e6270df0dc64d8b"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.3/scriba-aarch64-apple-darwin"
    sha256 "1e6966c5b1459d7615a7923abc2c608e03a5cf8da572f4dbe6282a02213b9f4c"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
