class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.19.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.0/scriba-x86_64-apple-darwin"
    sha256 "39ca84efede20d41a48ef390f636ab1655ae6a0302c1207931e46181b1351c47"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.0/scriba-aarch64-apple-darwin"
    sha256 "c01678808171a343e8814d1306a3e307d1af45c9b7bddfe8d5d09b4555d35a86"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
