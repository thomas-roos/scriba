class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.20.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.20.0/scriba-x86_64-apple-darwin"
    sha256 "cb26a7718669d4f10ed7a4e6a9a1a8455a6872b28e6b208a42be56378ae1db36"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.20.0/scriba-aarch64-apple-darwin"
    sha256 "1fb756f5e0d9fa4bfe53ec91a72b9e974d5284c261232c71d9e4eaa3807999b9"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
