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
 * MethodSelection.java
 *
 * A way of selecting between multiple methods/classes of input for a properties page.
 * e.g. Combo box to select "Int" or "String" and pages for each type's input
 *
 * @author Peter Halasz
 */

package net.pengo.propertyEditor;
import java.awt.FlowLayout;

import javax.swing.BoxLayout;
import javax.swing.JPanel;



public class MultiPage extends EditablePage {
    
    private PropertyPage[] page;
    private String name;
    
    private JPanel main;
    
	public MultiPage(PropertiesForm form, String name) {
		super(form);
        this.name = name;
		setForm(form);
	}
	
    public MultiPage(PropertiesForm form, PropertyPage[] page, String name) {
		this(form, name);
		
        setPages(page);
    }
	
	public void setPages(PropertyPage[] page) {
		this.page = page;
		setFormForPages();
		init();
		//build();
	}
    
    public void setForm(PropertiesForm form) {
        super.setForm(form); // this.form = form;
		
        setFormForPages();
    }
	
	private void setFormForPages() {
        if (page == null)
            return;
        
        for (int i = 0; i < page.length; i++) {
            page[i].setForm(form);
        }
	}
    
    public String toString() {
        return name;
    }
	
	//FIXME: dunno if i really need to override this.. but seems sensible.
	//FIXNE: move this to a superclass with "isAwareOfModding()" in subclasses
    public void save() {
        //if (modded) {
            saveOp();
        //}
    }
    protected void saveOp() {
        for (int i = 0; i < page.length; i++) {
            PropertyPage p = page[i];
            p.save();
        }
    }

	//initializes this page (but not subpages)
	protected void init() {
		removeAll();
		
        setLayout(new FlowLayout());
        
        main = new JPanel();
        
        BoxLayout layout = new BoxLayout(main, BoxLayout.Y_AXIS); //BoxLayout.Y_AXIS
		
        main.setLayout(layout);

        for (int i = 0; i < page.length; i++) {
            main.add(page[i]);
        }
        
        add(main);
        
        //main = new JPanel(layout);
        //add(main, BorderLayout.CENTER);
        
		//Not needed, as all sub pages already built.
        //build();
	}
	
    public void buildOp() {
        for (int i = 0; i < page.length; i++) {
            PropertyPage p = page[i];
            p.build();
        }
        
        // these needed?
        //revalidate();
        //repaint();
    }
    
    
    
}

