/*
 * DiffData.java
 *
 * Created on 31 August 2002, 13:30
 */
import java.io.*;
import java.util.*;
import java.lang.ref.*;

/**
 *
 * @author  administrator
 */
public class DiffData extends EditableData {
    protected OpenFile openFile;
    protected Data source;
    protected List modList;
    protected SortedSet breakList;
    
    protected LinkedList streamList;
    protected boolean streamFlow = true;
    
    protected List breakResList;
    
    /** Creates a new instance of DiffData */
    public DiffData(OpenFile openFile, Data source) {
        this.openFile = openFile;
        this.source = source;
        modList = new LinkedList();
        calcSourceBreaks();
    }
    
    public DiffData(Data source) {
        this(null, source);
    }
    
    public void setOpenFile(OpenFile openFile) {
        if (this.openFile != null) {
            clearBreakResList();
        }
        this.openFile = openFile;
    }
    
    public InputStream getDataStream() {
        return new DiffData.DiffDataInputStream();
    }
    
    public InputStream getDataStream(long offset) {
        try {
            return new DiffData.DiffDataInputStream(getStart() + offset, getStart() + getLength());
        } catch (IOException e) {
            e.printStackTrace(); //xxx: swallowed!
            return null;
        }
    }
    
    public InputStream getDataStream(long start, long length) {
        try {
            return new DiffData.DiffDataInputStream(start, length);
        } catch (IOException e) {
            e.printStackTrace(); //xxx: swallowed!
            return null;
        }
    }
    
    public void delete(long offset, long length) {
        addMod(new DelMod(offset, length));
    }

    public void delete(LongListSelectionModel selection) {
        //xxx: wrong wrong wrong
        addMod(new DelMod(selection.getMinSelectionIndex(), selection.getMaxSelectionIndex()));
    }
    
    public void insert(Data data) {
        addMod(new InsertMod(data));
    }

    public void overwrite(Data data) {
        addMod(new OverwriteMod(data));
    }
    
    public void insertReplace(LongListSelectionModel selection, Data newData) {
        //xxx: optimize with new Mod class
        super.insertReplace(selection,newData);
    }
    
    protected void addMod(Mod mod){
        //System.out.println("********* applying mod: " + mod);
        synchronized (breakList) {
            modList.add(mod);
            mod.apply();
        }
        debugOut();
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
        //xxx allow specific range to be calculated only
        
        breakList = Collections.synchronizedSortedSet(new TreeSet());
        breakList.add(source);
        
        for (int a = modList.size()-1; a >= 0; a--) {
            Mod mod = (Mod)modList.get(a);
            mod.apply();
        }
        
        debugOut();
    }
    
    // @returns true if a cut was made. false if cursor was on a gap.
    protected boolean cut(Cursor divider) {
        //debugOut("possible places to cut", breakList);
        //System.out.println("------- cutting: " + divider);
        SortedSet top = breakList.headSet(divider);
        debugOut("choose from", breakList);
        debugOut("narrow down to", top);

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
        debugOut("break list to delete from",breakList);
        SortedSet delRange = breakList.subSet(start, end);
        debugOut("delete",delRange);
        //breakList.removeAll(delRange);
        delRange.clear();
        
    }

    // move may be negative
    protected void moveEnd(Cursor start, long move) {
        //System.out.println("------- moving: " + start + " .. " + move + " bytes");
        cut(start);
        SortedSet moveRange = breakList.tailSet(start);
        debugOut("starting break list",breakList);
        debugOut("moving these", moveRange);
        TreeSet movedSet = new TreeSet();
        //debugOut("to move",moveRange);
        for (Iterator i = moveRange.iterator(); i.hasNext(); ) {
            Data data = (Data)i.next();
            Data movedData = data.getStartShiftedSelection(move);
            i.remove();
            movedSet.add(movedData);
        }
        //debugOut("moved",movedSet);
        breakList.addAll(movedSet);
        debugOut("moved", movedSet);
        debugOut("final break list",breakList);
    }
    
    public void debugOut(String name, Collection c) {
        //System.out.println(name + ": ");
        int a=0;
        for (Iterator i = c.iterator(); i.hasNext(); ) {
            Object o = i.next();
            //System.out.println(" - " + a + ": " + o);
            a++;
        }
    }
    public synchronized void debugOut() {
        
        if (breakResList == null) {
            breakResList = new LinkedList();
        } else {
            clearBreakResList();
        }
         
        if (openFile == null)
            return;
        
        for (Iterator i = breakList.iterator(); i.hasNext(); ) {
            Object o = i.next();
            //DefaultDefinitionResource ddr = new DefaultDefinitionResource(openFile, (Data)o);
            Data d = (Data)o;
            DefaultDefinitionResource ddr = new DefaultDefinitionResource(openFile, new SimpleLongListSelectionModel(d.getStart(), d.getStart()+d.getLength()));
            
            openFile.addBreak(this, ddr);
            breakResList.add(ddr);
            //a++;
        }
        
        //debugOut("Current mod List", modList);
        //debugOut("BreakList", breakList);
        
    }
    
    public void clearBreakResList() {
        if (openFile == null)
            return;
        
        for (Iterator i = breakResList.iterator(); i.hasNext(); ) {
            DefaultDefinitionResource ddr = (DefaultDefinitionResource)i.next();
            openFile.deleteBreak(this, ddr);
            i.remove();
        }
    }
    
    
    class DiffDataInputStream extends InputStream {
        Iterator breakIt;
        InputStream stream;
        long pos;
        long cdsrem; // current data stream's remaining bytes.
        Data currentData; // here only for debug
        //xxx: should we really need to check if we've run out out bytes via .getLength() ?? this is only needed for TransparentData's laziness-- ignoring of spec length 
        
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
            breakIt = breakList.iterator(); //xxx no good.
            pos = getStart();
        }

        public int read() throws java.io.IOException {
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
        protected boolean nextStream() {
            synchronized (breakList) { //xxx this wont really help
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
    
    abstract class Mod {
        abstract public void apply();
    }
    
    class InsertMod extends Mod {
        Data data;

        /** use data's start */
        public InsertMod(Data data) {
            this.data = data;
        }
        
        public void apply() {
            DiffData.this.moveEnd(new Cursor(data.getStart()), data.getLength());
            DiffData.this.breakList.add(data);
        }
        
        public String toString() {
            return "InsertMod data: " + data;
        }
        
        
    }

    class DelMod extends Mod {
        long offset;
        long deleteLength;
        
        /** delete */
        public DelMod(long offset, long deleteLength) {
            this.offset = offset;
            this.deleteLength = deleteLength;
        }
        
        public void apply() {
            Cursor endCur = new Cursor(offset+deleteLength);
            DiffData.this.deleteRange(new Cursor(offset), endCur);
            DiffData.this.moveEnd(endCur, -deleteLength);
        }

        public String toString() {
            return "DelMod offset: " + offset + " deleteLength: " + deleteLength;
        }
    }
    class OverwriteMod extends Mod {
        Data data;

        /** use data's start */
        public OverwriteMod(Data data) {
            this.data = data;
        }
        
        public void apply() {
            DiffData.this.deleteRange(new Cursor(data.getStart()), new Cursor(data.getStart()+data.getLength()));
            DiffData.this.breakList.add(data);
        }

        public String toString() {
            return "OverwriteMod data: " + data;
        }
    }
}
