class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.15.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.15.0/scriba-x86_64-apple-darwin"
    sha256 "65b742e52a03efff4c64b86aa54f8eaf173053fb447ef55b7e1a1c3e6063e788"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.15.0/scriba-aarch64-apple-darwin"
    sha256 "efca3138005ecb269653b439413eb1890eef816eb12ec7cadecd2777af559524"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
