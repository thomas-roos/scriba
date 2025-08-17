class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.5.8"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.5.8/scriba-x86_64-apple-darwin"
    sha256 "eab3f96149cf1853c6463c643acd3c2aac20ad4ae3d560abf17be734fb8fa5a9"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.5.8/scriba-aarch64-apple-darwin"
    sha256 "fd1c6d90c5af53f2889502bda77ed560f9df19dd8115ef497f020e995c0c38a0"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
