package net.pengo.resource;
import java.awt.event.ActionEvent;
import java.util.*;
import javax.swing.AbstractAction;
import javax.swing.Action;
import javax.swing.JMenu;
import net.pengo.app.OpenFile;
import net.pengo.data.Data;
import net.pengo.data.DemoData;
import net.pengo.data.DiffData;
import net.pengo.restree.ResourceList;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.selection.Segment;

public class LiveSelectionResource extends SelectionResource {
    public LiveSelectionResource(OpenFile openFile) {
	super(openFile);
    }
    
    public LongListSelectionModel getSelectionModel() {
	return openFile.getSelectionModel();
    }
    
    public JMenu getJMenu() {
	final SelectionResource This = this;
	
	JMenu menu = new JMenu("Selection");
	Action addToTemplate = new AbstractAction("Add to template") {
	    public void actionPerformed(ActionEvent e) {
		LongListSelectionModel selection = (LongListSelectionModel)(getOpenFile().getSelectionModel().clone());
		DefaultDefinitionResource def = new DefaultDefinitionResource(getOpenFile(), selection);
		//getOpenFile().addDefinition(this, def);
		getOpenFile().getDefinitionList().add(def);
		System.out.println("added");
	    }
	};
	menu.add(addToTemplate);
	
	final Data data = getOpenFile().getData();
	if (data instanceof DiffData) {
	    Action delAction = new AbstractAction("Delete selected hex") {
		public void actionPerformed(ActionEvent e) {
		    //getOpenFile().clearSelection(this);
		    //((DiffData)data).delete(sel.getStart(), sel.getLength());
		    LongListSelectionModel selection = getOpenFile().getSelectionModel();
		    getOpenFile().getEditableData().delete(selection);
		}
	    };
	    menu.add(delAction);
	    
	    //FIXME: does not require a selection, just a lead selection index
	    Action insAction = new AbstractAction("Insert 00randomFF (len:64)") {
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
	return menu;
    }
    
    
    //FIXME: redundant for live selection. selection is already selected.. isn't it?
    public void doubleClickAction() {
	super.doubleClickAction();
	LongListSelectionModel sel = getSelectionModel();
	getOpenFile().setSelectionModel(sel);
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
    
    public String toString() {
	return "Selection";
    }
    
    public synchronized void valueChanged(LongListSelectionEvent e) {
	LongListSelectionModel sel = getSelectionModel();
	List details = getOpenFile().getSelectionDetails();
	//fixme: adds to a stupid place.
	
	if (e.getValueIsAdjusting()) {
	    return;
	}
	
	long c = sel.getSegmentCount();
	details.clear();
	
	Segment[] seg = sel.getSegments();
	for (int i=0; i < seg.length; i++) {
	    details.add(seg[i]);
	}

    }
}

