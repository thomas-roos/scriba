class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.9.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.9.1/scriba-x86_64-apple-darwin"
    sha256 "a420b0e129af6529c717b05f0b96b81bdac338510cb820a90670621f2f69063a"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.9.1/scriba-aarch64-apple-darwin"
    sha256 "3a38e5449705221b052419e1726d2c328c3c3646cc241dfb009ab735505ff23d"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
