class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.11.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.11.1/scriba-x86_64-apple-darwin"
    sha256 "ca634cf3927574355ed6448277d18e624d8bdde6400a95b1d4eda302688d0d44"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.11.1/scriba-aarch64-apple-darwin"
    sha256 "c4a1f3ffd7262b75937cb1d2300878457562ad000a1f95c7807035db2df7d9a5"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
