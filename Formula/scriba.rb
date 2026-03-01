class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.21.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.21.0/scriba-x86_64-apple-darwin"
    sha256 "f950ebda508d8548663684c577dc8e3692e6e79d2a859e8f9b21151ca96fae3b"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.21.0/scriba-aarch64-apple-darwin"
    sha256 "9d4e2e8692c3e9bd348cf31377f926444bde78219e9c43b1e88c0b3b7e2e211e"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
