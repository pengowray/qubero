package net.pengo.resource;

import java.awt.event.ActionEvent;
import java.util.List;

import javax.swing.AbstractAction;
import javax.swing.Action;
import javax.swing.JMenu;
import javax.swing.JSeparator;

import net.pengo.app.ClipboardEvent;
import net.pengo.app.FileEvent;
import net.pengo.app.OpenFile;
import net.pengo.app.OpenFileListener;
import net.pengo.app.SelectionEvent;
import net.pengo.data.Data;
import net.pengo.data.DataEvent;
import net.pengo.data.DemoData;
import net.pengo.data.DiffData;
import net.pengo.data.SelectionData;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.selection.Segment;

public class LiveSelectionResource extends SelectionResource implements OpenFileListener, Cloneable
{
    
    private SelectionData selData; // cache thing
    
    public LiveSelectionResource(OpenFile openFile) {
        super(openFile);
        openFile.addLongListSelectionListener(this);
    }
    
    public LongListSelectionModel getSelection() {
        return openFile.getSelectionModel();
    }    
    
    public Resource[] getSources()
    {
        //  xxx:
        return new Resource[]{};
    }
    public SelectionData getSelectionData() {
        if (selData == null)
            selData = new SelectionData(getSelection(), openFile.getData());
        
        return selData;
    }
    
    public DefaultSelectionResource toDefaultSelectionResource(){
        return new DefaultSelectionResource(openFile, (LongListSelectionModel)getSelection().clone());
    }
    
    public void giveActions(JMenu menu) {
        super.giveActions(menu);
        menu.add(new JSeparator());

        TypeRegistry.instance().giveConversionActions(menu, toDefaultSelectionResource());
        menu.add(new JSeparator());
        /*
        Action addToTemplate = new AbstractAction("Add to template (ugh! replace!!)") {
            public void actionPerformed(ActionEvent e) {
                LongListSelectionModel selection = (LongListSelectionModel)(getOpenFile().getSelectionModel().clone());
                DefaultDefinitionResource def = new DefaultDefinitionResource(toDefaultSelectionResource());
                //getOpenFile().addDefinition(this, def);
                getOpenFile().getDefinitionList().add(def);
            }
        };
        menu.add(addToTemplate);
        */
        
        final Data data = getOpenFile().getData();
        if (data instanceof DiffData) {
            Action delAction = new AbstractAction("Delete selected hex") {
                public void actionPerformed(ActionEvent e) {
                    //getOpenFile().clearSelection(this);
                    //((DiffData)data).delete(sel.getStart(), sel.getLength());
                    getOpenFile().getEditableData().delete(getSelection());
                }
            };
            menu.add(delAction);
            
            //FIXME: does not require a selection, just a lead selection index
            Action insAction = new AbstractAction("Insert 00-random-FF (len:64)") {
                public void actionPerformed(ActionEvent e) {
                    //getOpenFile().clearSelection(this);
                    //((DiffData)data).insert( sel.getStart(), new DemoData((int)sel.getLength())); //FIXME: possible precision loss
                    long pos = getOpenFile().getSelectionModel().getLeadSelectionIndex();
                    getOpenFile().getEditableData().insert(pos, new DemoData(64));
                }
            };
            menu.add(insAction);
            
            //FIXME: put this back:
            /*
            Action replaceAction = new AbstractAction("replace with 00randomFF") {
                public void actionPerformed(ActionEvent e) {
                    getOpenFile().clearSelection(this);
                    ((DiffData)data).overwrite( sel.getStart(), new DemoData((int)sel.getLength())); //FIXME: possible precision loss
                }
            };
            menu.add(replaceAction);
            */
            
            //FIXME: paste
        }
        
        
        //JPopupMenu popup = menu.getPopupMenu();
    }
    
    //FIXME: redundant for live selection. selection is already selected.. isn't it?
    public void doubleClickAction() {
        super.doubleClickAction();
        LongListSelectionModel sel = getSelection();
        //getOpenFile().setSelectionModel(sel);
        System.out.println(" - Selection details from selection: " + sel);
        System.out.println("   Min/Max: " + sel.getMinSelectionIndex() + "-" + sel.getMaxSelectionIndex());
        System.out.println("   Segments: " + sel.getSegmentCount());
        Segment[] seg = sel.getSegments();
        if (seg.length > 0) {
            System.out.println("   First and last segments: "
                    + seg[0].firstIndex + "-" + seg[0].lastIndex
                    + ", " + seg[seg.length-1].firstIndex + "-" + seg[seg.length-1].lastIndex);
        }
    }
    
    
    // how to respond to a rename event
    public void rename(String name) {
        //FIXME:
        return;
    }
    
    public void updated() {
        selData = null;
        
        LongListSelectionModel sel = getSelection();
        List details = getOpenFile().getSelectionDetails();
        //fixme: adds to a stupid place.
        
        //long c = sel.getSegmentCount();
        details.clear();
        
        Segment[] seg = sel.getSegments();
        for (int i=0; i < seg.length; i++) {
            details.add(seg[i]);
        }
    }
    

    
    public void selectionCleared(SelectionEvent e) {
        updated();
    }
    
    public void dataEdited(DataEvent e) {
        updated();
    }
    
    public void dataLengthChanged(DataEvent e) {}
    
    public void fileClosed(FileEvent e) {
        // what now?
    }
    
    public void fileSaved(FileEvent e) {}
    
    public void selectionCopied(ClipboardEvent e) {}
    
    public void selectionMade(SelectionEvent e) {
        updated();
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        System.out.println("now editing.. yeah.");
    }
    
}

