/**
 * DefaultSelectionResource.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.resource;

import net.pengo.app.OpenFile;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionModel;

public class DefaultSelectionResource extends LiveSelectionResource
{
	LongListSelectionModel sel;
		
	public DefaultSelectionResource(OpenFile openFile, LongListSelectionModel sel) {
		super(openFile);
		this.sel = sel;
    }
		
	public LongListSelectionModel getSelectionModel() {
		return sel;
	}
	
    public void valueChanged(LongListSelectionEvent e) {
    }
	
	
}

