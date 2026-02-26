class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.19.2"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.2/scriba-x86_64-apple-darwin"
    sha256 "89fa8423afc84f9e8839e42f2d15a44afe0c0630a0a05d3f260a9b7af510811a"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.19.2/scriba-aarch64-apple-darwin"
    sha256 "1f498dec463ccb540bc0d6dce22e88a84c6a9531780b22cc15dae3a6397c4c8c"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
