class Scriba < Formula
  desc "Modern CLI tool for recording and transcribing audio using OpenAI Whisper"
  homepage "https://github.com/giovannialberto/scriba"
  version "0.6.1"
  
  if Hardware::CPU.intel?
    url "https://github.com/giovannialberto/scriba/releases/download/v0.6.1/scriba-x86_64-apple-darwin"
    sha256 "49ef853048a66af2eaa806fa3668ac4686d47520d533f8d03eaace9865f1599a"
  else
    url "https://github.com/giovannialberto/scriba/releases/download/v0.6.1/scriba-aarch64-apple-darwin"
    sha256 "bf8d77b5ab0da8ae478ae4cadd546f610986c1fb58e774f173a49c26a15e4e93"
  end
  
  def install
    bin.install "scriba-#{Hardware::CPU.intel? ? "x86_64" : "aarch64"}-apple-darwin" => "scriba"
  end
  
  test do
    assert_match version.to_s, shell_output("#{bin}/scriba --version")
  end
end
