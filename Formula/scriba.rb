class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.19.4"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.4/scriba-x86_64-apple-darwin"
    sha256 "1427eb28d1653c73b92d39eacd531b3b18ea7e500e96ae9535aafa4c5c5d379d"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.4/scriba-aarch64-apple-darwin"
    sha256 "22f8765e39cae8d1bc55dfa7c45f2744e6392384c5ec2d4b9c52be0b630143f0"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
