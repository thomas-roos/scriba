class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.18.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.18.0/scriba-x86_64-apple-darwin"
    sha256 "9973dbc731ca780379d8d3b0713b45776527f5d5c195458be544ebbe688d0ceb"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.18.0/scriba-aarch64-apple-darwin"
    sha256 "941c0a5f379b36c3ec5f4a0c1ef641cdca3cfe4b6b5b1cfceb9e8d8e108f40b3"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
