class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.21.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.21.1/scriba-x86_64-apple-darwin"
    sha256 "7a8be454af14e768f1c8a9fd5699847b1201e33ecd52538dd6875fea947518c8"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.21.1/scriba-aarch64-apple-darwin"
    sha256 "0dd4c3a9e40e22174d02d304f5645a5805abec6abf38eefae867379562cf3af9"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
