/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * MagicNumberResource.java
 *
 * @author  Peter Halasz
 */

package net.pengo.resource;

import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;
import net.pengo.propertyEditor.ResourceForm;

public class MagicNumberResource extends DefinitionResource implements AddressedResource {

    // the selection within the template which should match the magic number
    public final SmartPointer sel = new JavaPointer("net.pengo.resource.SelectionResource");
    
    // the magic number itself
    public final SmartPointer magic = new JavaPointer("net.pengo.resource.SelectionResource"); 
    
    public MagicNumberResource(SelectionResource sel){
        this(sel, sel);
    
    }
            
    /**
     * @param openFile
     */
    public MagicNumberResource(SelectionResource sel, SelectionResource magic) {
        this.sel.setName("selection");
        this.sel.addSink(this);
        this.sel.setValue(sel);
        
        this.magic.setName("magic number");
        this.magic.addSink(this);
        this.magic.setValue(magic);
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.DefinitionResource#editProperties()
     */
    public void editProperties() {
        new ResourceForm(this).show();
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#getSources()
     */
    public Resource[] getSources() {
        return new Resource[] { sel, magic};
    }
    
    public JavaPointer[] getJPointers() {
        return new JavaPointer[]{(JavaPointer)sel, (JavaPointer)magic};
    }
    
    /* (non-Javadoc)
     * @see net.pengo.resource.AddressedResource#getSelectionResource()
     */
    public SelectionResource getSelectionResource() {
        return (SelectionResource)sel.evaluate();
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.AddressedResource#setSelectionResource(net.pengo.resource.SelectionResource)
     */
    public void setSelectionResource(SelectionResource selRes) {
        sel.setValue(selRes);
    }

    public void doubleClickAction() {
        super.doubleClickAction();
        getSelectionResource().makeActive();
    }
}

