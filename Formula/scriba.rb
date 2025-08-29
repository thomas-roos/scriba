class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.11.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.11.0/scriba-x86_64-apple-darwin"
    sha256 "31144c2ae162838a0d1a612025d515cf8adefe32ee245f395033e834d1ef28c4"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.11.0/scriba-aarch64-apple-darwin"
    sha256 "eab3585e6072cc7b0cf39ddb49f15e388722f1df151970cce5a613a60e7497e9"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
