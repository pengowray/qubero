import java.io.*;
import javax.swing.tree.*;

/* recursive defintion (naming/mapping) of some binary stuff */
/* when making definitions, attempt to keep useful sequences together. e.g. if a parent node has two overlapping sets of sub-data, put each set in its own non-leaf node.*/

interface Chunk {

  // what sort of thing is this called within its spec?
  public String getType();
  
  // the name for the side menu
  //public String toString();
  
  // what sort of thing is this? (for working out the icon?)
  //public int getTypeID();
  
  /** returns the all chunk(s) within this one*/
  //public Chunk[] pos();

  /** returns the chunk(s) at this position within this chunk */
  //public Chunk[] pos(int start); // will throw "OutOfBounds" exception

  /** returns the chunk(s) between these positions within this chunk */
  //public Chunk[] pos(int start, int length); // will throw "OutOfBounds" exception

  /** launch an editor for this entire chunk. */
  //public void edit(); // throw EditorNotFound except in future 

  /** can this entire chunk be edited / is there an editor for it? */
  //public boolean isEditable(); // XXX: make this an interface?
  
  //XXX: replace with iterators and stuff
  //public byte[] getData();
}




class RecordSetChunk {
  
}

class ByteChunk {

}

class FlagsChunk {

}

class NarrativeSequenceChunk {

}

class OverviewDisplayChunk {

}

