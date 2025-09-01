class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.12.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.12.1/scriba-x86_64-apple-darwin"
    sha256 "84e3cc706ffc987e2a3f62d1577bf5157132f143275aa7027253619b730edfee"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.12.1/scriba-aarch64-apple-darwin"
    sha256 "01ca5243d25d4d7611424702314ccaf7a3d696da03732790f8b49dec3424fab3"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
