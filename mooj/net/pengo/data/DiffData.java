/*
 * DiffData.java
 *
 * Created on 31 August 2002, 13:30
 */
package net.pengo.data;
import java.io.IOException;
import java.io.InputStream;
import java.util.Collections;
import java.util.Iterator;
import java.util.LinkedList;
import java.util.SortedSet;
import java.util.TreeSet;
import net.pengo.app.Cursor;
import net.pengo.app.OpenFile;
import net.pengo.resource.ResourceFactory;
import net.pengo.restree.ResourceList;
import net.pengo.restree.ResourceSortedSet;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.selection.Segment;

/**
 *
 * @author  administrator
 */
public class DiffData extends EditableData {
    protected OpenFile openFile;
    protected Data source;
    protected ResourceList modList;
    protected ResourceSortedSet breakList;
    
    //protected ResourceList resModList; // List
    //protected ResourceSortedSet resBreakList; // SortedSet

    protected LinkedList streamList;
    protected boolean streamFlow = true;
    
    /** Creates a new instance of DiffData */
    public DiffData(OpenFile openFile, Data source) {
        this.source = source;
	setOpenFile(openFile);
        calcSourceBreaks();
    }
    
    public DiffData(Data source) {
        this(null, source);
    }
    
    public void setOpenFile(OpenFile openFile) {
	if (breakList == null) {
	    breakList =
		new ResourceSortedSet(Collections.synchronizedSortedSet(new TreeSet()), openFile, "Breaks");
	}
	
	if (modList == null) {
	    modList =
		new ResourceList(new LinkedList(), openFile, "Undo Wishlist");
	}
	
        if (this.openFile != null) {
            //fixme
            openFile.getResourceList().remove(modList);
            openFile.getResourceList().remove(breakList);
	}
        this.openFile = openFile;

        if (openFile != null) {
            openFile.getResourceList().add(modList);
            openFile.getResourceList().add(breakList);
	}
	
	modList.setResourceFactory(openFile);
	breakList.setResourceFactory(openFile);

    }
    
    public InputStream dataStream() {
        return new DiffData.DiffDataInputStream();
    }
    
    public InputStream getDataStream(long offset) {
        try {
            return new DiffData.DiffDataInputStream(getStart() + offset, getStart() + getLength());
        } catch (IOException e) {
            e.printStackTrace(); //FIXME: swallowed!
            return null;
        }
    }
    
    public InputStream getDataStream(long start, long length) {
        try {
            return new DiffData.DiffDataInputStream(start, length);
        } catch (IOException e) {
            e.printStackTrace(); //FIXME: swallowed!
            return null;
        }
    }
    
    public void delete(long offset, long length) {
        addMod(new DelMod(offset, length));
    }

    public void delete(LongListSelectionModel selection) {
        //fixme: should be combined into a "super mod" so undos can happen more seamlessly
        Segment[] seg = selection.getSegments();
        for (int i=0; i<seg.length; i++) {
            addMod(new DelMod(seg[i].firstIndex, seg[i].lastIndex - seg[i].firstIndex +1 ));
        }
    }
    
    public void insert(Data data) {
        addMod(new InsertMod(data));
    }

    public void overwrite(Data data) {
        addMod(new OverwriteMod(data));
    }
    
    public void insertReplace(LongListSelectionModel selection, Data newData) {
        //FIXME: optimize with new Mod class
        super.insertReplace(selection,newData);
    }
    
    protected void addMod(Mod mod){
        synchronized (breakList) {
            modList.add(mod);
            mod.apply();
        }
        fireDataUpdated(this, mod);
    }


    /*
    protected void freezeStreams() {
        
        for (Iterator i = streamList.iterator(); i.hasNext(); ){
            WeakReference wr = (WeakReference)i.next();
            Object stream = wr.get();
            if (stream != null) {
                DiffDataInputStream ddis = (DiffDataInputStream)stream;
                ddis.lock();
            } else {
                i.remove();
            }
        }
    }
     */
    
    /** first piece of data is where? */
    public long getStart() {
        Data first = (Data)breakList.first();
        return first.getStart();
    }

    public long getLength() {
        Data last = (Data)breakList.last();
        return (last.getStart() + last.getLength()) - getStart();
    }
    
    public String getType() {
        return "diff data";
    }
    
    public String toString() {
        return source + "";
        
        /* // this is lame
        if (modList.isEmpty()) {
            return source + "";
        } else {
            return source + "*";
        }
        */
    }
    
    /** calculates where the places source changes from one source to another */
    protected synchronized void calcSourceBreaks() {
        //FIXME: allow specific range to be calculated only
        
        breakList.clear();
        breakList.add(source);
        
        for (int a = modList.size()-1; a >= 0; a--) {
                Mod mod = (Mod)modList.get(a);
                mod.apply();
        }
        
        //debugOut();
    }
    
    // @returns true if a cut was made. false if cursor was on a gap.
    protected boolean cut(Cursor divider) {
        //debugOut("possible places to cut", breakList);
        //System.out.println("------- cutting: " + divider);
        SortedSet top = breakList.headSet(divider);
        //debugOut("choose from", breakList);
        //debugOut("narrow down to", top);

        Data maySplit;
        if (top.isEmpty()) {
            //System.out.println("empty list so no cut");
            return false; // nothing to left of divider
        }

        
        maySplit = (Data)top.last();
        //System.out.println("maysplit: " + maySplit);
        long maySplitEnd = maySplit.getStart() + maySplit.getLength();
        if (maySplitEnd <= divider.getStartBefore()) { // divider is in a gap.
            //System.out.println("wont split because maySplitEnd is: " + maySplitEnd + " which is <= " + divider.getStartBefore());
            return false;
        }
        
        // time to cut maySplit.
        breakList.remove(maySplit);
        
        Data leftSplit = maySplit.getSelection(maySplit.getStart(), divider.getStartBefore()-maySplit.getStart());
        breakList.add(leftSplit);
        
        Data rightSplit = maySplit.getSelection(divider.getStartBefore(), (maySplit.getStart() + maySplit.getLength()) - divider.getStartBefore() );
        breakList.add(rightSplit);
        
        //System.out.println("cutting:" + maySplit + " @" + divider + " left:" + leftSplit + " right:" + rightSplit);
        return true;
    }
    
    // deletes range and leaves a gap in the breakList between start and end.
    protected void deleteRange(Cursor start, Cursor end) {
        cut(start);
        cut(end);
        //System.out.println("------- deleting: " + start + " -to- " + end);
        //debugOut("break list to delete from",breakList);
        SortedSet delRange = breakList.subSet(start, end);
        //debugOut("delete",delRange);
        //breakList.removeAll(delRange);
        delRange.clear();
        
    }

    // move may be negative
    protected void moveEnd(Cursor start, long move) {
        //System.out.println("------- moving: " + start + " .. " + move + " bytes");
        cut(start);
        SortedSet moveRange = breakList.tailSet(start);
        //debugOut("starting break list",breakList);
        //debugOut("moving these", moveRange);
        TreeSet movedSet = new TreeSet();
        //debugOut("to move",moveRange);
        for (Iterator i = moveRange.iterator(); i.hasNext(); ) {
            Data data = (Data)i.next();
            Data movedData = data.shiftStart(move);
            i.remove();
            movedSet.add(movedData);
        }
        //debugOut("moved",movedSet);
        breakList.addAll(movedSet);
        //debugOut("moved", movedSet);
        //debugOut("final break list",breakList);
    }
    
    class DiffDataInputStream extends InputStream {
        Iterator breakIt;
        InputStream stream;
        long pos;
        long cdsrem; // current data stream's remaining bytes.
        Data currentData; // here only for debug
        //FIXME: should we really need to check if we've run out out bytes via .getLength() ?? this is only needed for TransparentData's laziness-- ignoring of spec length
        
        // start is relative to the board.
        public DiffDataInputStream(long start, long length) throws IOException {
            pos = start;
            Cursor cur = new Cursor(start);
            breakIt = breakList.tailSet(cur).iterator();
            SortedSet curDataSet = breakList.headSet(cur);
            if (!curDataSet.isEmpty()) {
                currentData = (Data)curDataSet.last();
                cdsrem = currentData.getStart() + currentData.getLength() - pos;
                stream = currentData.getDataStream(start,cdsrem);
            } else {
                // else is handled by nextStream()
                currentData = null;
                cdsrem = 0;
            }
        }
        
        public DiffDataInputStream() {
            //streamList.add(new WeakReference(this));
            breakIt = breakList.iterator(); //FIXME: no good.
            pos = getStart();
        }

        public int read() throws IOException {
            // get first stream
             if (stream == null && nextStream() == false) {
                    return -1;
             }
             
             if (cdsrem <= 0) { //  || ret == -1
                if (nextStream() == false) {
                    return -1;
                }
                return read();
             }
            
             int ret = stream.read();
             
             if (ret == -1) {
                System.out.println("read too far (or file error)! so much for counting.\nsegment:" + currentData + " pos:" + pos + " remaining:" + cdsrem);
             }
            
             pos++;
             cdsrem--;
             return ret;
        }
        
        /** go to the next stream on the list.
         * @returns true on success.
         * @returns false if no more. */
        protected boolean nextStream() throws IOException {
            synchronized (breakList) { //FIXME: this wont really help
                if (!breakIt.hasNext())
                    return false;
                currentData = (Data)breakIt.next(); // get the new/next data stream.
                //stream = currentData.getDataStream(); // stream already knows where it should be
                stream = currentData.getDataStream(pos,currentData.getStart() + currentData.getLength() - pos);
                cdsrem = currentData.getLength();

                // error check. remove later?
                if (currentData.getStart() != pos) {
                    System.out.println("Segment " + currentData + " starts at wrong position: " + currentData.getStart() + " .. expected " + pos);
                }

                return true;
            }
        }
    }
    
    public abstract class Mod {
        protected abstract void apply();
    }
    
    public class InsertMod extends Mod {
        Data data;

        /** use data's start */
        public InsertMod(Data data) {
            this.data = data;
        }
        
        protected void apply() {
            DiffData.this.moveEnd(new Cursor(data.getStart()), data.getLength());
            DiffData.this.breakList.add(data);
        }
        
        public String toString() {
            return "InsertMod data: " + data;
        }
        
        
    }

    public class DelMod extends Mod {
        long offset;
        long deleteLength;
        
        /** delete */
        public DelMod(long offset, long deleteLength) {
            //fixme: should be start and end (not length)
            this.offset = offset;
            this.deleteLength = deleteLength;
        }
        
        protected void apply() {
            Cursor endCur = new Cursor(offset+deleteLength);
            DiffData.this.deleteRange(new Cursor(offset), endCur);
            DiffData.this.moveEnd(endCur, -deleteLength);
        }

        public String toString() {
            return "DelMod offset: " + offset + " deleteLength: " + deleteLength;
        }
    }
    public class OverwriteMod extends Mod {
        Data data;

        /** use data's start */
        public OverwriteMod(Data data) {
            this.data = data;
        }
        
        protected void apply() {
            DiffData.this.deleteRange(new Cursor(data.getStart()), new Cursor(data.getStart()+data.getLength()));
            DiffData.this.breakList.add(data);
        }

        public String toString() {
            return "OverwriteMod data: " + data;
        }
    }
}
