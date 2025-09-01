class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.12.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.12.0/scriba-x86_64-apple-darwin"
    sha256 "5e85336929485e988eb3445d219d2dd871061351719cbaa38b6d2764a5db2e4d"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.12.0/scriba-aarch64-apple-darwin"
    sha256 "b75265054174a9d6e0289a094c99137cd9087a49df66b402e7463b543304831e"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
