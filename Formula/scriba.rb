class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.5.5"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.5.5/scriba-x86_64-apple-darwin"
    sha256 "940b679f276eccc506d549b25b9c37a8eed3f0ad8e0c690f22d3c44cbdaf35d9"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.5.5/scriba-aarch64-apple-darwin"
    sha256 "a5ca036a10b0ad0c02b3cf241750f762ed013c2ff7c892f9b5b1b8d528caa090"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
