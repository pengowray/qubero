/**
 * MethodSelection.java
 *
 * A way of selecting between multiple methods/classes of input for a properties page.
 * e.g. Combo box to select "Int" or "String" and pages for each type's input
 *
 * @author Created by Omnicore CodeGuide
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

