class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.21.2"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.21.2/scriba-x86_64-apple-darwin"
    sha256 "6e9ede73a5ce74133a854da1c1c731c38f413fb85f61b5ba6dc38fcb21127456"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.21.2/scriba-aarch64-apple-darwin"
    sha256 "8896aae2f0ab75b204df19cb2cb9efe02f4b6e4a93f15ea1174207a6cd70164c"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
